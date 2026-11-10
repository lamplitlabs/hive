"use client";

// Settings -> Git (ADR-070 step 3): whether PULSE reads and writes
// repositories as a GitHub App, read from GET /api/v1/git-app. The three
// settings themselves are the group's own rows below this strip; the words
// live in lib/git-app-state.ts so node --test can check them.
import { ExternalLinkIcon, GitBranchIcon } from "lucide-react";
import { Notice } from "@/components/pulse/panel";
import { gitAppView, NEW_APP_URL, repoCount, type GitAppPayload } from "@/lib/git-app-state";
import { settingLabel } from "@/lib/setting-labels";
import { useApi } from "@/lib/use-api";
import { cn } from "@/lib/utils";

const hint = "text-[13px] leading-snug text-muted-foreground anywhere";
const link = "inline-flex items-center gap-1 text-[13px] font-medium text-primary underline-offset-4 hover:underline";

export function GitAppStrip() {
  const q = useApi<GitAppPayload>("git-app", { refresh: 60000 });
  if (q.error) return <p className={cn(hint, "px-4 py-4 sm:px-5")}>PULSE couldn&apos;t read the GitHub App status: {q.error}</p>;
  const v = gitAppView(q.data);
  if (!v) return null;
  return (
    <section className="grid gap-3 px-4 py-4 sm:px-5" aria-label="GitHub App">
      <div className="grid gap-0.5">
        <h3 className="flex items-center gap-1.5 text-sm font-semibold">
          <GitBranchIcon className="size-4 text-muted-foreground" aria-hidden />
          GitHub App
        </h3>
        <p className={hint}>PULSE keeps the App&apos;s ids and where its private key lives, never the key. Each token lasts an hour and opens one repository.</p>
      </div>
      <Notice tone={v.tone}>
        <p className="font-medium">{v.headline}</p>
        <p className="text-muted-foreground">{v.detail}</p>
      </Notice>
      {v.missing.length && v.tone === "info" ? (
        <div className="grid gap-1">
          <p className={hint}>Still to fill in below: {v.missing.map((n) => settingLabel(n)).join(", ")}.</p>
          <a className={link} href={NEW_APP_URL} target="_blank" rel="noreferrer">
            Register a GitHub App
            <ExternalLinkIcon className="size-3.5" aria-hidden />
          </a>
        </div>
      ) : null}
      {v.repositories.length ? (
        <details className="group">
          <summary className={cn(hint, "cursor-pointer select-none")}>Covers {repoCount(v.repositories.length)}</summary>
          <ul className="mt-1.5 grid gap-0.5 font-mono text-xs">
            {v.repositories.map((r) => (
              <li key={r} className="truncate">
                {r}
              </li>
            ))}
          </ul>
        </details>
      ) : null}
      {v.installUrl ? (
        <a className={link} href={v.installUrl} target="_blank" rel="noreferrer">
          Add or remove repositories
          <ExternalLinkIcon className="size-3.5" aria-hidden />
        </a>
      ) : null}
    </section>
  );
}
