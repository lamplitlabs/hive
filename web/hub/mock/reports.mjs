// The mock /api/v1/reports: the done tasks of mock/api/reports.json, newest
// first, filtered by project and paged by limit the way pulse-hub's
// reports.rs answers (unknown project 404, a bad limit 400, at most 200).
export function createReportsMock(fixture, projects = []) {
  const reports = [...(fixture.reports || [])].sort((a, b) => String(b.done_at).localeCompare(String(a.done_at)));
  const known = new Set(projects.length ? projects : reports.map((r) => r.project));
  return {
    read(q) {
      const project = q.get("project") || "";
      if (project && !known.has(project)) return { status: 404, body: { error: "no such project", why: project + " is not registered in the mock store", fix: "Choose a registered project." } };
      const raw = q.get("limit");
      let limit = 50;
      if (raw !== null && raw !== "") {
        if (!/^\d+$/.test(raw) || Number(raw) <= 0 || Number(raw) > 200) return { status: 400, body: { error: "invalid limit", why: "limit must be a whole number from 1 to 200", fix: "Use limit=200 or less." } };
        limit = Number(raw);
      }
      const scoped = reports.filter((r) => !project || r.project === project);
      return { status: 200, body: { scope: { project, limit, total: scoped.length, has_more: scoped.length > limit }, reports: scoped.slice(0, limit) } };
    },
  };
}
