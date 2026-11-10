"use client";

import { Suspense, useCallback } from "react";
import { usePathname, useRouter, useSearchParams } from "next/navigation";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { WorkReports } from "@/components/reports/work-reports";
import { PageHeader } from "@/components/hive/page-header";
import { ProjectAvatar } from "@/components/hive/avatar";
import { SkeletonPage } from "@/components/hive/panel";
import { registeredProjects, type Org } from "@/lib/api";
import { useApi } from "@/lib/use-api";

export default function WorkReportsPage() {
  return (
    <Suspense fallback={<SkeletonPage rows={3} />}>
      <WorkReportsScreen />
    </Suspense>
  );
}

function WorkReportsScreen() {
  const sp = useSearchParams();
  const router = useRouter();
  const pathname = usePathname();
  const project = sp.get("project") || "";
  const org = useApi<Org>("org", { refresh: 60000 });
  const projects = registeredProjects(org.data);
  const setProject = useCallback(
    (slug: string) => {
      const q = new URLSearchParams(sp.toString());
      if (slug) q.set("project", slug);
      else q.delete("project");
      const s = q.toString();
      router.push(pathname + (s ? "?" + s : ""), { scroll: false });
    },
    [sp, router, pathname],
  );
  return (
    <>
      <PageHeader
        title="Work reports"
        subtitle="Every finished task: what landed, in which release, and the evidence"
        action={
          <div className="flex items-center gap-2">
            <Select value={project || "__all"} onValueChange={(v) => setProject(v === "__all" ? "" : v)}>
              <SelectTrigger aria-label="Project" className="h-9 w-48">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="__all">All projects</SelectItem>
                {projects.map((p) => (
                  <SelectItem key={p.slug} value={p.slug}>
                    <span className="flex items-center gap-2">
                      <ProjectAvatar slug={p.slug} name={p.name} size={20} />
                      {p.name || p.slug}
                    </span>
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            {org.error ? <span role="alert" className="text-[13px] text-status-failed">{"The project list didn't load: " + org.error}</span> : null}
          </div>
        }
      />
      <WorkReports key={project || "__all"} project={project || undefined} />
    </>
  );
}
