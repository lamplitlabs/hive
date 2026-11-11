// Pure helpers of Settings -> Git (ADR-070 step 3): what the panel says
// about the GitHub App PULSE reads and writes repositories as, from the
// GET /api/v1/git-app payload. No React, no "@/" imports, so
// git-app-state.test.mjs drives it under node --test.

export interface GitAppPayload {
  available: boolean;
  configured: boolean;
  app_id?: string;
  installation_id?: string;
  private_key_ref?: string;
  app?: { slug: string; name: string; html_url: string } | null;
  install_url?: string;
  account?: { login: string; type: string } | null;
  repository_selection?: string;
  repositories?: string[];
  error?: string;
}

export type GitAppTone = "ok" | "warn" | "info";

export interface GitAppView {
  tone: GitAppTone;
  headline: string;
  detail: string;
  // The settings still empty, by name, in the order the steps fill them.
  missing: string[];
  repositories: string[];
  installUrl: string;
}

export const GIT_APP_SETTINGS = ["git.github_app.app_id", "git.github_app.installation_id", "git.github_app.private_key_ref"] as const;

// GitHub's page for registering a new App; the owner's first step.
export const NEW_APP_URL = "https://github.com/settings/apps/new";

export function gitAppView(p: GitAppPayload | undefined): GitAppView | null {
  if (!p || !p.available) return null;
  const missing = GIT_APP_SETTINGS.filter((name) => {
    const key = name.split(".").pop() as "app_id" | "installation_id" | "private_key_ref";
    return !(p[key] ?? "").trim();
  });
  const repositories = p.repositories ?? [];
  const installUrl = p.install_url ?? "";
  if (!p.configured) {
    return {
      tone: "info",
      headline: "Git uses this computer's own logins",
      detail: "Install a GitHub App once and every job gets a one-hour token for its one repository instead. No key or login stays on the machines that run the work.",
      missing,
      repositories: [],
      installUrl,
    };
  }
  if (p.error) {
    return {
      tone: "warn",
      headline: "PULSE can't reach the GitHub App",
      detail: p.error + ". Until it can, git and gh use this computer's own logins.",
      missing,
      repositories,
      installUrl,
    };
  }
  const who = p.account?.login ? `${p.account.login}` : "your account";
  const name = p.app?.name || p.app?.slug || "The GitHub App";
  const scope = p.repository_selection === "all" ? "every repository of " + who : repoCount(repositories.length) + " of " + who;
  return {
    tone: "ok",
    headline: `${name} is installed on ${who}`,
    detail: `Jobs push and open pull requests with a one-hour token for their one repository. It covers ${scope}.`,
    missing,
    repositories,
    installUrl,
  };
}

export function repoCount(n: number): string {
  return n === 1 ? "1 repository" : `${n} repositories`;
}
