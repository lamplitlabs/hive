"use client";

import Link from "next/link";
import { useRouter } from "next/navigation";
import { FolderIcon, TargetIcon } from "lucide-react";
import { Button } from "@/components/ui/button";
import { AddProjectButton } from "@/components/project/project-dialogs";
import { GOAL_ANCHOR } from "@/components/project/goal";
import { ProjectAvatar } from "@/components/pulse/avatar";
import { ListRow, ListRows } from "@/components/pulse/list-row";
import { SectionHeading } from "@/components/pulse/section-heading";
import type { ProjectCardData } from "@/lib/api";
import { rel } from "@/lib/format";
import { hrefs } from "@/lib/nav";
import { useNow } from "@/lib/use-api";
import { cn } from "@/lib/utils";
import { projectLine } from "./home-state";

// "Projects" (spec "Home" 6): one compact row per project - avatar, name and
// one line: how many things wait on the owner (marigold), else what is
// wrong, else when its board last moved. The row opens the project; "Add"
// sits in the heading. A project still on a generated goal gets a "Set goal"
// link on its row that leads to the goal (what the old nudge banners did).
export function ProjectsList({ projects, waiting, onRegistered }: { projects: ProjectCardData[]; waiting: (slug: string) => number; onRegistered?: (slug: string) => void }) {
  const now = useNow(60000);
  return (
    <section aria-labelledby="home-projects" className="min-w-0">
      <SectionHeading id="home-projects" icon={FolderIcon} count={projects.length} action={<AddProjectButton size="xs" variant="ghost" onRegistered={(r) => onRegistered?.(r.project.slug)} />}>
        Projects
      </SectionHeading>
      {projects.length ? (
        <ListRows ariaLabel="Projects">
          {projects.map((p) => {
            const n = waiting(p.slug);
            const line = projectLine(p, n, (d) => rel(d, now));
            return (
              <ListRow
                key={p.slug}
                lead={<ProjectAvatar slug={p.slug} name={p.name} size={32} />}
                title={p.name || p.slug}
                href={hrefs.project(p.slug)}
                ariaLabel={(p.name || p.slug) + ": " + line.text}
                // The row is a link already, so the cue is a span the row's own
                // click handles; the goal anchor is a separate row-level link only
                // when the project has no other news.
                status={line.setGoal ? <SetGoal slug={p.slug} /> : undefined}
              >
                <span className={cn(line.tone === "you" && "font-medium text-you-ink", line.tone === "bad" && "text-status-failed")}>{line.text}</span>
              </ListRow>
            );
          })}
        </ListRows>
      ) : (
        <p className="rounded-lg border border-dashed px-4 py-3 text-sm text-muted-foreground">No project yet. Add one to give the agents something to work on.</p>
      )}
    </section>
  );
}

// "Set goal" on a project row. The row is an anchor, so this is a button
// that navigates (an <a> inside the row's <a> would be invalid HTML).
function SetGoal({ slug }: { slug: string }) {
  const router = useRouter();
  return (
    <Button
      size="xs"
      variant="ghost"
      className="text-link"
      onClick={(e) => {
        e.preventDefault();
        e.stopPropagation();
        router.push(hrefs.project(slug) + "#" + GOAL_ANCHOR);
      }}
    >
      <TargetIcon /> Set goal
    </Button>
  );
}

// A quiet link under a column when there is nothing else to say.
export function SeeInsights() {
  return (
    <Link href={hrefs.insights()} className="text-sm text-link hover:underline">
      See insights
    </Link>
  );
}
