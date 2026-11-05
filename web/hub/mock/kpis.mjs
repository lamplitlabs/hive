// GET /api/v1/kpis offline: the fixture holds 60 days; the mock answers the
// window the query asks for (`days`, default 30, at most 90) and 404s a
// project the mock store does not know, like the hub does.

const invalid = (why) => ({ status: 400, body: { error: "Invalid days", why, fix: "Use days=1..90." } });

// Every counted field of a day, in the payload's order; `cost` is the one
// that is not a whole number.
export const KPI_KEYS = ["sessions", "prompt_tokens", "completion_tokens", "cost", "jobs_done", "jobs_failed", "landings", "tasks_done", "reviews", "reviews_merged", "findings", "vulns_found", "vuln_fixes", "owner_asks"];

export function createKpisMock(fixture, projects = []) {
  const known = new Set(projects);
  const series = fixture.series || [];
  return {
    read(q) {
      const project = q.get("project") || "";
      if (project && known.size && !known.has(project)) return { status: 404, body: { error: "no such project", why: project + " is not registered in the mock store", fix: "Choose a registered project." } };
      const raw = q.get("days");
      let days = 30;
      if (raw !== null && raw !== "") {
        if (!/^\d+$/.test(raw)) return invalid("days must be a positive whole number");
        days = Number(raw);
        if (days <= 0 || days > 90) return invalid("days must be between 1 and 90");
      }
      const rows = series.slice(Math.max(0, series.length - days)).map((r) => (project ? scaled(r, project) : r));
      const totals = Object.fromEntries(KPI_KEYS.map((k) => [k, 0]));
      for (const r of rows) for (const k of KPI_KEYS) totals[k] += Number(r[k] || 0);
      return {
        status: 200,
        body: {
          now: fixture.now,
          days,
          from: rows[0]?.day || fixture.to,
          to: rows[rows.length - 1]?.day || fixture.to,
          project,
          series: rows,
          totals,
          priced: totals.cost > 0,
        },
      };
    },
  };
}

// A project's share of the fleet's day: a stable fraction per slug, so two
// projects differ and the same project reads the same on every reload.
// Merged reviews shrink with reviews so the share never exceeds the whole.
function scaled(r, project) {
  let h = 0;
  for (const c of project) h = (h * 31 + c.charCodeAt(0)) >>> 0;
  const share = 0.2 + (h % 50) / 100;
  const part = (n) => Math.round(Number(n || 0) * share);
  const out = { ...r };
  for (const k of KPI_KEYS) out[k] = k === "cost" ? Number(r.cost || 0) * share : part(r[k]);
  out.reviews_merged = Math.min(out.reviews_merged, out.reviews);
  return out;
}
