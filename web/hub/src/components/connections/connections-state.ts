// Pure helpers for the Connections page (spec "Connections"): a tool's
// state as an icon + word, its tool count, where a skill applies and when
// it was last used. No React and no "@/" imports so node --test can load
// it straight.

export type ConnectionKind = "connected" | "sign_in" | "failed" | "unchecked" | "off";

// What a tool card says about a connection, as the StatusBadge wants it:
// `status` is a word statusKind maps (connected → done, waiting → waits
// on you in marigold, neutral → grey Off, failed, pending → queued "not
// checked yet") and `label` the words beside the icon.
export function connectionState(c: { status?: string | null; auth?: string | null; last_check_at?: string | null; last_check_ok?: boolean | null; last_check_note?: string | null; sign_in?: { signed_in?: boolean | null; note?: string | null } | null }): { kind: ConnectionKind; status: string; label: string; detail: string } {
  const detail = ((c.last_check_note || "").split("\n")[0] ?? "").trim();
  if ((c.status || "").toLowerCase() === "disabled") return { kind: "off", status: "neutral", label: "Off", detail };
  if (c.auth === "oauth" && c.sign_in && c.sign_in.signed_in === false) return { kind: "sign_in", status: "waiting", label: "Needs sign-in", detail: c.sign_in.note || detail };
  if (c.last_check_ok === false) {
    if (c.auth === "oauth" && /hive mcp login|sign in|signed in|unauthori[sz]ed|401/i.test(detail)) return { kind: "sign_in", status: "waiting", label: "Needs sign-in", detail };
    return { kind: "failed", status: "failed", label: "Failed", detail };
  }
  if (c.last_check_ok === true) return { kind: "connected", status: "connected", label: "Connected", detail };
  return { kind: "unchecked", status: "pending", label: "Not checked yet", detail };
}

// How many tools the last check found ("name 1.2: 12 tools: a, b"); null
// when the note does not say.
export function toolCount(note: string | null | undefined): number | null {
  const m = /\b(\d+)\s+tools?\b/i.exec(note || "");
  if (m) return Number(m[1]);
  const list = /\btools?:\s*([^]+)$/.exec(note || "");
  if (!list) return null;
  return list[1].split(",").map((s) => s.trim()).filter(Boolean).length;
}

// "12 tools", "1 tool", or "" when the count is unknown.
export function toolCountText(note: string | null | undefined): string {
  const n = toolCount(note);
  if (n === null) return "";
  return n + (n === 1 ? " tool" : " tools");
}

// The projects a connection or skill serves: [] is every project ("" or
// "*"); a comma-separated list is the named ones.
export function servesProjects(project: string | null | undefined): string[] {
  const p = (project || "").trim();
  if (!p || p === "*") return [];
  return p
    .split(",")
    .map((s) => s.trim())
    .filter(Boolean);
}

// When a skill was last used, in words: skills carry a use count and an
// updated_at, not a last-used time, so "Used 3 times" with the update time
// is the closest truth. `uses` 0 is "Not used yet".
export function skillUseText(k: { uses?: number | null }): string {
  const n = k.uses ?? 0;
  if (!n) return "Not used yet";
  if (n === 1) return "Used once";
  return "Used " + n + " times";
}

// The suggestions row shows each tool once, even when several projects
// want it; the first project's reason stays and the others are counted.
export function dedupeSuggestions<T extends { name: string; project: string }>(rows: T[]): (T & { also: string[] })[] {
  const seen = new Map<string, T & { also: string[] }>();
  for (const r of rows) {
    const key = r.name.toLowerCase();
    const have = seen.get(key);
    if (have) {
      if (r.project && !have.also.includes(r.project) && r.project !== have.project) have.also.push(r.project);
    } else seen.set(key, { ...r, also: [] });
  }
  return [...seen.values()];
}

// Suggestions already connected (a server of the same name, in the same
// project or shared) are not suggested again.
export function unconnectedSuggestions<T extends { name: string; project: string }>(rows: T[], servers: { name: string; project: string }[]): T[] {
  const have = new Set(servers.map((s) => s.name.toLowerCase() + "\u0000" + (s.project || "")));
  const shared = new Set(servers.filter((s) => !s.project).map((s) => s.name.toLowerCase()));
  return rows.filter((r) => !shared.has(r.name.toLowerCase()) && !have.has(r.name.toLowerCase() + "\u0000" + (r.project || "")));
}
