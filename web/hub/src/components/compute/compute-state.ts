// Pure helpers for the Machines page (spec "Machines"): the meters on the
// hero card, the request row's title, what a machine's card shows. No React
// and no "@/" imports so node --test can load it straight.

export type MeterTone = "ok" | "warning" | "failed";

// A meter's tint by how full it is: green until 70 %, marigold until 90 %,
// red past that. Anything not a finite number is "ok" (the bar is empty).
export function meterTone(pct: number | null | undefined): MeterTone {
  if (pct === null || pct === undefined || !Number.isFinite(pct)) return "ok";
  if (pct >= 90) return "failed";
  if (pct >= 70) return "warning";
  return "ok";
}

// A used-of-total pair as a 0-100 percentage; null when the total is nothing.
export function meterPercent(used: number | null | undefined, total: number | null | undefined): number | null {
  if (used === null || used === undefined || total === null || total === undefined) return null;
  if (!Number.isFinite(used) || !Number.isFinite(total) || total <= 0) return null;
  return Math.max(0, Math.min(100, Math.round((used / total) * 100)));
}

// A request for a machine every project can use names the shared board
// (hive_tasks::ANY_PROJECT): the brain pairs its machine for `*`. The
// console never shows that slug for a request; it says "Any project".
export const ANY_PROJECT = "commons";
export const ANY_PROJECT_WORD = "Any project";

// Whose work a request's machine takes, as the console words it: the
// project's name (its slug when no name is known), "Any project" for a
// machine every project can use, "" when the request names none.
export function requestProject(project: string | null | undefined, nameOf?: (slug: string) => string | null | undefined): string {
  const slug = (project || "").trim();
  if (!slug) return "";
  if (slug === ANY_PROJECT) return ANY_PROJECT_WORD;
  return (nameOf?.(slug) || "").trim() || slug;
}

// The line under the form's project list: what the choice means for the
// machine. `name` is the chosen project's name; "" while nothing is chosen.
export function requestProjectHint(project: string, name?: string): string {
  if (!project) return "Whose work it runs. Any project lets every project use it.";
  if (project === ANY_PROJECT) return "Every project's work can run on it.";
  return "Only " + ((name || "").trim() || project) + "'s work runs on it.";
}

// The request row's title: "Extra machine for 8 h · demo-lib", or
// "… · Any project" for a machine every project can use. Hours are
// whole when they are; a half hour keeps one decimal.
export function requestTitle(req: { hours: number; project: string }): string {
  const h = Number.isFinite(req.hours) ? req.hours : 0;
  const hours = Number.isInteger(h) ? String(h) : String(Math.round(h * 10) / 10);
  const project = requestProject(req.project);
  return "Extra machine for " + hours + " h" + (project ? " · " + project : "");
}

// The runner's one-word state for the hero badge, from the heartbeat's
// state and the owner's pause (`runner.allowed` false means the stop
// switch is on). "stopped" is a daemon that never ran, died or is unknown.
export type RunnerState = "running" | "paused" | "stopped";

export function runnerState(r: { daemon?: { state?: string } | null; runner?: { allowed: boolean } | null } | null | undefined): RunnerState {
  if (!r) return "stopped";
  if (r.runner && !r.runner.allowed) return "paused";
  const s = (r.daemon?.state || "").toLowerCase();
  if (s === "running" || s === "alive") return "running";
  return "stopped";
}

export const RUNNER_STATE_WORD: Record<RunnerState, string> = { running: "Running", paused: "Paused", stopped: "Stopped" };

// The system a machine runs, from its `offers` line ("macos arm64 xcode"),
// its probe note or kind. Null when nothing says.
export type MachineOs = "macos" | "linux" | "windows" | null;

export function machineOs(e: { offers?: string | null; kind?: string | null; probe_note?: string | null; name?: string | null }, hostHint?: string | null): MachineOs {
  const words = (e.offers || "").toLowerCase().split(/\s+/);
  if (words.includes("macos")) return "macos";
  if (words.includes("linux")) return "linux";
  if (words.includes("windows")) return "windows";
  const text = ((e.probe_note || "") + " " + (hostHint || "")).toLowerCase();
  if (/\bmac(os|book)?\b|darwin/.test(text)) return "macos";
  if (/\b(linux|ubuntu|debian)\b/.test(text)) return "linux";
  if (/\bwindows\b/.test(text)) return "windows";
  // Over ssh the runner assumes Linux (executor.rs machine_caps does the same).
  if ((e.kind || "").toLowerCase() === "ssh") return "linux";
  return null;
}

// This machine's system: what its own row or the host's name says, else what
// the browser runs on (the console is mostly opened on the machine itself).
// The "This machine" card and the local machine's card in "All machines"
// share it, so the one machine wears one mark.
export function thisMachineOs(local: { offers?: string | null; kind?: string | null; probe_note?: string | null; name?: string | null } | null | undefined, host?: string | null, userAgent?: string | null): MachineOs {
  return machineOs(local ?? { offers: "", kind: "local", probe_note: "this machine" }, host) ?? (/mac/i.test(userAgent || "") ? "macos" : null);
}

// The brand mark for a machine card: the system for a local/ssh machine,
// the cloud provider for one that was requested. Returns a logoFor() key
// (a word logos.ts matches by substring) or null for the Server icon.
export function machineLogoKey(e: { kind?: string | null; offers?: string | null; probe_note?: string | null; name?: string | null }, provider?: string | null): string | null {
  const kind = (e.kind || "").toLowerCase();
  if (/cloud|azure|aws|gcp|hetzner|vm|request/.test(kind)) {
    const p = (provider || "").toLowerCase().trim();
    if (p === "gcp" || p === "google") return "google cloud";
    if (p) return p;
    return kind;
  }
  const os = machineOs(e);
  if (os === "macos") return "macos";
  if (os === "linux") {
    const words = (e.offers || "").toLowerCase();
    if (/\bubuntu\b/.test(words)) return "ubuntu";
    if (/\bdebian\b/.test(words)) return "debian";
    return "linux";
  }
  return null;
}

// The labels a machine offers, without its system and chip words (those
// are the logo) and the "(reserved)" marker (that is its own chip).
const SYSTEM_WORDS = new Set(["macos", "linux", "windows", "arm64", "amd64", "-"]);

export function machineLabels(offers: string | null | undefined): string[] {
  return (offers || "")
    .replace("(reserved)", "")
    .split(/\s+/)
    .map((w) => w.trim())
    .filter((w) => w && !SYSTEM_WORDS.has(w.toLowerCase()));
}

// The projects a machine serves: [] means every project ("*" or empty).
export function machineProjects(projects: string | null | undefined): string[] {
  const p = (projects || "").trim();
  if (!p || p === "*") return [];
  return p
    .split(",")
    .map((s) => s.trim())
    .filter(Boolean);
}

// The slots meter: how many of the machine's agent slots are in use, and the
// words "1 of 4 in use". Used slots fill the bar, so the words count the same
// thing the bar shows (TASK-173: it said "4 of 4 free" beside an empty bar).
export function slotsMeter(e: { workers: number; running?: number; free?: number }): { used: number; total: number; pct: number | null; text: string } {
  const total = Math.max(0, e.workers || 0);
  const free = e.free === undefined || e.free === null ? Math.max(0, total - (e.running ?? 0)) : Math.max(0, Math.min(total, e.free));
  const used = Math.max(0, total - free);
  return { used, total, pct: meterPercent(used, total), text: total ? used + " of " + total + " in use" : "No slots" };
}

// The daemon's "disk: 252 MB free, needs 1024; 6 projects waiting" line as
// a meter: how much of the floor is gone. The runner reports disk only while
// headroom is short, so an absent line is "—" (no data), not full.
export function diskMeter(line: string | null | undefined): { pct: number | null; text: string } {
  const m = /disk:\s*(\d+)\s*MB free,\s*needs\s*(\d+)/i.exec(line || "");
  if (!m) return { pct: null, text: "—" };
  const free = Number(m[1]);
  const floor = Number(m[2]);
  if (!Number.isFinite(free) || !Number.isFinite(floor) || floor <= 0) return { pct: null, text: "—" };
  const pct = Math.max(0, Math.min(100, Math.round((1 - free / floor) * 100)));
  return { pct, text: free + " MB free" };
}

// The keep-awake switch position from the daemon.keep_awake setting: "on"
// and "always" are on, "off" (or unset) is off.
export function keepAwakeOn(mode: string | null | undefined): boolean {
  const m = (mode || "").trim().toLowerCase();
  return m === "on" || m === "always";
}

// A short reason for a request row: the first sentence, clipped to one line.
export function requestReason(reason: string | null | undefined, max = 120): string {
  const r = (reason || "").trim().replace(/\s+/g, " ");
  if (!r) return "";
  const first = r.split(/(?<=[.!?])\s/)[0] || r;
  return first.length > max ? first.slice(0, max - 1).trimEnd() + "…" : first;
}

// The line under "Waiting for you" on the Machines page. Two things can wait
// there: an agent's request for an extra machine, and Hive's own question
// about cleaning up after the other agent programs. The line says which, so
// it does not name a request when only the question waits.
export function waitingNote(requests: number, cleanupQuestion: boolean): string {
  if (requests > 0) return "An agent asked for an extra machine. It starts only if you approve.";
  return cleanupQuestion ? "Hive asks before it cleans up after other agent programs on this machine." : "";
}

// ---- TASK-165: the owner's own rental is never refused by a limit ----

// The limit a request passes, as the brain counts it (hive_tasks::LimitHit).
export type LimitHit = { setting: string; limit: number; used: number; asks: number; raise_to: number };

// What the brain says before the owner rents (GET compute/preview).
export type RentalPreview = {
  fit: { fits: number; waiting: number; blocked: { project: string; jobs: number; missing: string[] }[] };
  fit_words: string;
  limit: LimitHit | null;
  limit_words: string;
  os_needs: string[];
  linux_risk: string;
};

export type RentChoice = { id: "rent" | "raise" | "keep-off" | "linux"; label: string; recommended: boolean; detail?: string };

// The answers on a request over a limit, the same in the rent dialog and on a
// refused request's row: Rent it anyway (Recommended), Rent it and raise the
// limit to N, Do not rent. No free text.
export function limitChoices(hit: LimitHit): RentChoice[] {
  return [
    { id: "rent", label: "Rent it anyway", recommended: true, detail: "Your own request; the limit stays as it is for agents and the daemon." },
    { id: "raise", label: "Rent it and raise the limit to " + hit.raise_to, recommended: false, detail: limitName(hit.setting) + " becomes " + hit.raise_to + "." },
    { id: "keep-off", label: "Do not rent", recommended: false },
  ];
}

// The choices before an owner's rental (item 3) when no waiting job could run
// on the machine: Do not rent (Recommended), Rent it anyway, and - when the
// project's needs name an OS a rented Linux machine lacks - Let project jobs
// run on Linux, with its risk. Null when a waiting job fits, or none waits.
export function fitChoices(p: Pick<RentalPreview, "fit" | "os_needs" | "linux_risk">): RentChoice[] | null {
  if (p.fit.fits > 0 || p.fit.blocked.length === 0) return null;
  const out: RentChoice[] = [
    { id: "keep-off", label: "Do not rent", recommended: true },
    { id: "rent", label: "Rent it anyway", recommended: false, detail: "It sits idle and is released after 30 minutes unless a job it can run arrives." },
  ];
  if (p.os_needs.length) out.push({ id: "linux", label: "Let project jobs run on Linux", recommended: false, detail: "Risk: " + p.linux_risk + "." });
  return out;
}

export function choiceLabel(c: RentChoice): string {
  return c.label + (c.recommended ? " (Recommended)" : "");
}

export function limitName(setting: string): string {
  return setting === "compute.max_vms" ? "Machines at once" : setting === "compute.max_hours_per_day" ? "Machine hours per day" : setting;
}

const NONE = /^(none|off|unlimited)$/i;

// The line beside a rental limit's field (item 2): 0 is explained, none is its own choice.
export function limitFieldHelp(value: string | null | undefined): string {
  const v = (value ?? "").trim();
  if (NONE.test(v)) return "No limit - agents and the daemon rent as they need.";
  if (v === "0") return "0 - Hive rents nothing. Your own requests still go through.";
  return "Agents and the daemon rent up to this; your own requests are never refused by it. 0 - Hive rents nothing.";
}

// Said after a change of a limit that leaves no rental possible for agents and
// the daemon (item 2); "" otherwise.
export function noRentalWarning(maxVms: string | null | undefined, maxHours: string | null | undefined): string {
  const z = (v: string | null | undefined) => (v ?? "").trim() === "0";
  if (z(maxVms) && z(maxHours)) return "Both limits are 0: Hive rents no machine by itself. Your own requests still go through.";
  if (z(maxVms)) return "Machines at once is 0: Hive rents no machine by itself, whatever the hours say. Your own requests still go through.";
  if (z(maxHours)) return "Machine hours per day is 0: Hive rents no machine by itself, whatever the machine count says. Your own requests still go through.";
  return "";
}

// The offer after a limit change, when requests were refused today: "" when none.
export function refileOffer(refused: number): string {
  if (refused <= 0) return "";
  return refused === 1 ? "1 request was not rented today because of a limit. Rent it now?" : refused + " requests were not rented today because of a limit. Rent them now?";
}

// How a settled request reads on the Machines page (item 4): one a limit refused is "Not
// rented" - its note says which limit and how much was used - never "Failed".
export function requestState(r: { status: string; note?: string | null; limit_hit?: LimitHit | null }): { badge: string; label?: string; lead: string; bad: boolean } {
  const note = (r.note ?? "").trim();
  if (r.limit_hit || /^Not rented: /.test(note)) return { badge: "waits", label: "Not rented", lead: "", bad: false };
  if (r.status === "failed") return { badge: "failed", lead: "Why it failed: ", bad: true };
  return { badge: r.status, lead: "", bad: false };
}
