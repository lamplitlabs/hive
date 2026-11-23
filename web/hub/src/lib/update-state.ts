// "A newer Hive is available" (TASK-087): the banner over every page when
// the daemon's daily check (setting update.latest_url) found a release newer
// than the running build. The hub decides `newer` with the same semver
// comparison `hive version` uses (crates/hive-cli/src/update.rs); this
// module only turns the payload into what the banner says, and remembers in
// this browser which release the owner dismissed, so the notice comes back
// for the next release but not on every page load for this one.
// Pure: no DOM, no fetch; update-banner.tsx owns the hook and localStorage.
//
// "Update now" (TASK-093): on update.channel release the banner offers to
// install the release through the hub (POST update/apply), which runs what
// `hive update` runs in the background; `apply` in the payload follows it.
// Once it is in place the runner and the console restart on it when their
// runs finish - the banner then shows the runner's own wait words
// (daemonUpdateText in components/hive/daemon-control-state.ts), handed in
// as `waitText` so this module stays free of runtime imports.
//
// Where a package manager installed Hive (Homebrew, for one) there is no
// button: that manager replaces the program, and the banner names its
// upgrade command (`managed_by`, `install_command` in the payload).
import type { UpdatePayload } from "./api";

export const UPDATE_DISMISS_KEY = "hive.update.dismissed";

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
    title: `Hive ${latest} is available`,
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

// How often the banner asks again while an install runs, and otherwise.
export const UPDATE_POLL_INSTALLING_MS = 2000;
export const UPDATE_POLL_IDLE_MS = 15 * 60_000;

export type UpdateApplyTone = "info" | "ok" | "bad";

export interface UpdateApplyView {
  // Whether the "Update now" button shows, and what it says.
  button: string | null;
  // The button shows but cannot be pressed (an install is running).
  busy: boolean;
  // The tooltip that says what pressing it does.
  buttonTitle: string;
  // One line under the title about the install, or null.
  line: string | null;
  tone: UpdateApplyTone | null;
  // Poll fast while the hub installs.
  installing: boolean;
}

export interface UpdateApplyInput {
  // The POST is in flight.
  posting?: boolean;
  // The POST was refused (a 409 on source, a 403...): its words.
  postError?: string;
  // The runner's own wait phrase once the release is in place ("A new
  // version waits for ..."), or "" when it announced none.
  waitText?: string;
}

// What the banner says and offers about installing `p.latest`. An apply
// state about another release than the one on the banner is old news and
// reads as idle.
export function updateApplyView(p: UpdatePayload | null | undefined, input: UpdateApplyInput = {}): UpdateApplyView {
  const none: UpdateApplyView = { button: null, busy: false, buttonTitle: "", line: null, tone: null, installing: false };
  if (!p) return none;
  const latest = (p.latest ?? "").trim();
  const apply = p.apply;
  const applyLatest = (apply?.latest ?? "").trim();
  const ours = !!apply && (!applyLatest || applyLatest === latest);
  const state = ours ? apply.state : "idle";
  const release = p.channel === "release";
  // A package manager's program is that manager's to replace: the hub
  // refuses to install over it, so the banner offers no button and names
  // the manager's own command, whatever the channel.
  const managedBy = (p.managed_by ?? "").trim();
  if (managedBy) {
    const command = (p.install_command ?? "").trim();
    return {
      ...none,
      line: command
        ? `${managedBy} installed Hive on this machine. Upgrade it in a terminal: ${command}`
        : `${managedBy} installed Hive on this machine and upgrades it.`,
      tone: "info",
    };
  }
  const buttonTitle = `Download ${latest || "the release"}, check it against the release's SHA256SUMS and install it. The runner and this console restart on it once their runs finish.`;
  if (input.posting || state === "installing") {
    return { button: "Installing…", busy: true, buttonTitle, line: `Downloading and checking ${applyLatest || latest || "the release"}…`, tone: "info", installing: true };
  }
  if (state === "installed") {
    const wait = (input.waitText ?? "").trim().replace(/\.$/, "");
    const what = applyLatest || latest;
    return {
      ...none,
      line: wait
        ? `Installed ${what}. ${wait}.`
        : `Installed ${what}. The runner and this console restart on it once their runs in flight finish.`,
      tone: "ok",
    };
  }
  if (input.postError) {
    return { ...none, button: release ? "Try again" : null, buttonTitle, line: `Not started: ${input.postError}`, tone: "bad" };
  }
  if (state === "failed") {
    const why = (apply?.error ?? "").trim() || "it stopped without saying why";
    return { ...none, button: release ? "Try again" : null, buttonTitle, line: `The update did not install: ${why}`, tone: "bad" };
  }
  if (state === "current") {
    return { ...none, line: `Nothing to install: the release source names ${applyLatest || latest}, which this build already runs.`, tone: "info" };
  }
  if (p.channel === "source") {
    return { ...none, line: "This machine builds Hive from source (update.channel), so the runner updates itself after each landing.", tone: null };
  }
  if (release && latest) return { ...none, button: "Update now", buttonTitle };
  return none;
}
