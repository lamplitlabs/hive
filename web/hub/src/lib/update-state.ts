// "A newer PULSE is available" (TASK-087): the banner over every page when
// the daemon's daily check (setting update.latest_url) found a release newer
// than the running build. The hub decides `newer` with the same semver
// comparison `pulse version` uses (crates/pulse-cli/src/update.rs); this
// module only turns the payload into what the banner says, and remembers in
// this browser which release the owner dismissed, so the notice comes back
// for the next release but not on every page load for this one.
// Pure: no DOM, no fetch; update-banner.tsx owns the hook and localStorage.
import type { UpdatePayload } from "./api";

export const UPDATE_DISMISS_KEY = "pulse.update.dismissed";

// The subset of Storage the banner uses, so tests can pass a Map-backed one.
export interface UpdateStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

export interface UpdateBanner {
  // The release, for the dismiss key and the heading.
  latest: string;
  title: string;
  detail: string;
  // The release page, only when it is an http(s) link.
  href: string | null;
}

export function webLink(url: string | null | undefined): string | null {
  const u = (url ?? "").trim();
  if (!/^https?:\/\/\S+$/i.test(u)) return null;
  try {
    const p = new URL(u);
    return p.protocol === "https:" || p.protocol === "http:" ? u : null;
  } catch {
    return null;
  }
}

// What the banner shows, or null when it should not show: the check is off,
// nothing newer is known, or the owner dismissed this very release.
export function updateBanner(p: UpdatePayload | null | undefined, dismissed: string | null = null): UpdateBanner | null {
  if (!p || !p.enabled || !p.newer) return null;
  const latest = (p.latest ?? "").trim();
  if (!latest) return null;
  if (dismissed && dismissed === latest) return null;
  const running = (p.running ?? "").trim();
  return {
    latest,
    title: `PULSE ${latest} is available`,
    detail: running ? `This machine runs ${running}.` : "A newer release than this machine's is out.",
    href: webLink(p.url),
  };
}

export function readDismissed(storage: UpdateStorage | null): string | null {
  try {
    return storage?.getItem(UPDATE_DISMISS_KEY) || null;
  } catch {
    return null;
  }
}

export function writeDismissed(storage: UpdateStorage | null, latest: string): void {
  try {
    storage?.setItem(UPDATE_DISMISS_KEY, latest);
  } catch {
    // private mode or storage disabled: the banner simply hides until reload
  }
}
