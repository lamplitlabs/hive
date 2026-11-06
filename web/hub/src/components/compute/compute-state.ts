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

// The request row's title: "Extra machine for 8 h · demo-lib". Hours are
// whole when they are; a half hour keeps one decimal.
export function requestTitle(req: { hours: number; project: string }): string {
  const h = Number.isFinite(req.hours) ? req.hours : 0;
  const hours = Number.isInteger(h) ? String(h) : String(Math.round(h * 10) / 10);
  const project = (req.project || "").trim();
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

// The slots meter: how many of the machine's agent slots are free, and the
// words "4 of 4 free". Used slots fill the bar.
export function slotsMeter(e: { workers: number; running?: number; free?: number }): { used: number; total: number; pct: number | null; text: string } {
  const total = Math.max(0, e.workers || 0);
  const free = e.free === undefined || e.free === null ? Math.max(0, total - (e.running ?? 0)) : Math.max(0, Math.min(total, e.free));
  const used = Math.max(0, total - free);
  return { used, total, pct: meterPercent(used, total), text: total ? free + " of " + total + " free" : "No slots" };
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
