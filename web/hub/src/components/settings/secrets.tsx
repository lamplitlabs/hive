"use client";

import { useState } from "react";
import { KeyRoundIcon, Loader2Icon, PlusIcon, Trash2Icon } from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { ConfirmDialog, showApiError } from "@/components/pulse/confirm";
import { ErrorNotice, Notice } from "@/components/pulse/panel";
import { Mono, When } from "@/components/pulse/who";
import { writes, type SecretEntry, type SecretsPayload } from "@/lib/api";
import { invalidate, useApi } from "@/lib/use-api";
import { MASK, canSaveSecret, normalizeSecretName, secretNameProblem, setByWords, vaultNote } from "./secrets-state";

// Settings → Secrets: the team secret vault (ADR-070 rule 4). Names, who set
// each and when; a value is typed once, sealed on the brain and never shown
// back. A job gets the names its project's environment recipe lists under
// `secrets:`, as environment variables for that run only.
const refresh = () => invalidate((p) => p === "secrets" || p.startsWith("events"));

export function SecretsSection({ canEdit }: { canEdit: boolean }) {
  const q = useApi<SecretsPayload>("secrets", { refresh: 30000 });
  const [adding, setAdding] = useState(false);
  const [removing, setRemoving] = useState<SecretEntry | null>(null);
  const list = q.data?.secrets ?? [];

  return (
    <div className="grid min-w-0 gap-3 rounded-lg border bg-card p-4">
      <p className="text-[13px] leading-relaxed text-muted-foreground">{q.data ? vaultNote(list.length) : "Loading the vault…"}</p>
      {q.error && !q.data ? (
        <ErrorNotice error={q.error} onRetry={() => void q.reload()}>
          {"The secrets didn't load (" + q.error + "). Check that PULSE is running, then try again."}
        </ErrorNotice>
      ) : null}
      {list.length ? (
        <ul className="grid min-w-0 divide-y rounded-md border" aria-label="Secrets in the team vault">
          {list.map((s) => (
            <li key={s.name} className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1 px-3 py-2">
              <KeyRoundIcon aria-hidden className="size-4 shrink-0 text-muted-foreground" />
              <Mono className="shrink-0">{s.name}</Mono>
              <span className="font-mono text-[13px] tracking-widest text-muted-foreground" aria-label="value hidden">
                {MASK}
              </span>
              <span className="min-w-0 flex-1 text-[13px] text-muted-foreground">
                {setByWords(s.updated_by)} <When d={s.updated_at} />
              </span>
              {canEdit ? (
                <Button variant="ghost" size="sm" onClick={() => setRemoving(s)} aria-label={"Remove " + s.name}>
                  <Trash2Icon aria-hidden /> Remove
                </Button>
              ) : null}
            </li>
          ))}
        </ul>
      ) : null}
      {canEdit ? (
        adding ? (
          <SecretForm maxBytes={q.data?.max_bytes ?? 0} taken={list.map((s) => s.name)} onDone={() => setAdding(false)} />
        ) : (
          <div>
            <Button variant="outline" size="sm" onClick={() => setAdding(true)}>
              <PlusIcon aria-hidden /> Add a secret
            </Button>
          </div>
        )
      ) : (
        <Notice tone="info">Only the owner adds or removes secrets.</Notice>
      )}
      <ConfirmDialog
        open={!!removing}
        onOpenChange={(o) => (o ? null : setRemoving(null))}
        title={"Remove " + (removing?.name ?? "") + "?"}
        description="Jobs that need it start without it until you add it again. The value cannot be brought back."
        action="Remove"
        destructive
        onConfirm={() => writes.removeSecret(removing?.name ?? "")}
        success={"Removed " + (removing?.name ?? "")}
        invalidates={(p) => p === "secrets" || p.startsWith("events")}
      />
    </div>
  );
}

function SecretForm({ maxBytes, taken, onDone }: { maxBytes: number; taken: string[]; onDone: () => void }) {
  const [name, setName] = useState("");
  const [value, setValue] = useState("");
  const [busy, setBusy] = useState(false);
  const clean = normalizeSecretName(name);
  const problem = secretNameProblem(clean);
  const replaces = !!clean && taken.includes(clean);
  const ok = canSaveSecret(clean, value, maxBytes) && !busy;

  const save = async () => {
    setBusy(true);
    try {
      await writes.setSecret(clean, value);
      setValue("");
      toast.success((replaces ? "Replaced " : "Saved ") + clean);
      refresh();
      onDone();
    } catch (e) {
      showApiError(e, "The secret wasn't saved");
    } finally {
      setBusy(false);
    }
  };

  return (
    <form
      className="grid min-w-0 gap-3 rounded-md border p-3"
      onSubmit={(e) => {
        e.preventDefault();
        if (ok) void save();
      }}
    >
      <div className="grid gap-1.5">
        <Label htmlFor="secret-name">Name</Label>
        <Input id="secret-name" value={name} onChange={(e) => setName(e.target.value)} placeholder="GITHUB_TOKEN" autoComplete="off" spellCheck={false} aria-invalid={!!problem} />
        <p className="text-[12px] text-muted-foreground" aria-live="polite">
          {problem || (replaces ? clean + " is in the vault; saving replaces its value." : clean && clean !== name ? "Saved as " + clean + "." : "The variable a job sees, in upper case.")}
        </p>
      </div>
      <div className="grid gap-1.5">
        <Label htmlFor="secret-value">Value</Label>
        <Input id="secret-value" type="password" value={value} onChange={(e) => setValue(e.target.value)} autoComplete="new-password" spellCheck={false} />
        <p className="text-[12px] text-muted-foreground">Sealed on this machine as you save; nobody sees it here again.</p>
      </div>
      <div className="flex gap-2">
        <Button type="submit" size="sm" disabled={!ok}>
          {busy ? <Loader2Icon aria-hidden className="animate-spin" /> : null} Save
        </Button>
        <Button type="button" variant="ghost" size="sm" onClick={onDone}>
          Cancel
        </Button>
      </div>
    </form>
  );
}
