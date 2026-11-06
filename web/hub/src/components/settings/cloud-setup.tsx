"use client";

// The cloud setup forms (ADR-070): where extra machines come from and where
// backups go, drawn from the registry GET /api/v1/clouds sends - a new cloud
// is a new descriptor in crates/pulse-store/src/clouds, not a new form. All
// state and rules live in lib/cloud-setup-state.ts; this file only renders.
import { useMemo, useState } from "react";
import { CheckIcon, CloudIcon, CopyIcon, DatabaseBackupIcon, KeyRoundIcon, Loader2Icon } from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { showApiError } from "@/components/pulse/confirm";
import { Notice } from "@/components/pulse/panel";
import { writes, type CloudField, type CloudsPayload } from "@/lib/api";
import { authMode, backupTo, changes, cloudOf, initialDraft, missingFields, problems, readiness, signInCommand, visibleFields, type CloudDraft } from "@/lib/cloud-setup-state";
import { invalidate, useApi } from "@/lib/use-api";
import { cn } from "@/lib/utils";

const hint = "text-[13px] leading-snug text-muted-foreground anywhere";

function useDraft(p: CloudsPayload | undefined) {
  const [draft, setDraft] = useState<CloudDraft | null>(null);
  const [seen, setSeen] = useState("");
  // A fresh payload (after a save, or another tab's) resets the form; the
  // reset happens while rendering, so no frame shows the stale draft.
  const stamp = p ? JSON.stringify([p.provider, p.clouds.map((c) => c.values), p.backup.to, p.backup.values]) : "";
  if (p && stamp !== seen) {
    setSeen(stamp);
    setDraft(initialDraft(p));
  }
  return [draft, setDraft] as const;
}

async function save(list: [string, string][], what: string) {
  for (const [name, value] of list) {
    try {
      await writes.setSetting(name, value);
    } catch (e) {
      showApiError(e, "PULSE couldn't save " + name + ".");
      invalidate((x) => x.startsWith("clouds") || x.startsWith("settings"));
      return false;
    }
  }
  toast.success(what);
  invalidate((x) => x.startsWith("clouds") || x.startsWith("settings") || x.startsWith("compute"));
  return true;
}

function Choice({ name, options, value, onPick, disabled }: { name: string; options: { id: string; label: string; blurb: string }[]; value: string; onPick: (id: string) => void; disabled: boolean }) {
  return (
    <div role="radiogroup" aria-label={name} className="grid gap-2 sm:grid-cols-2">
      {options.map((o) => {
        const on = o.id === value;
        return (
          <button
            key={o.id}
            type="button"
            role="radio"
            aria-checked={on}
            disabled={disabled}
            onClick={() => onPick(o.id)}
            className={cn("grid gap-0.5 rounded-lg border px-3 py-2.5 text-left text-sm transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-60", on ? "border-primary bg-primary/[0.06]" : "hover:bg-muted/40")}
          >
            <span className="flex items-center gap-1.5 font-medium">
              {on ? <CheckIcon className="size-3.5 text-primary" aria-hidden /> : null}
              {o.label}
            </span>
            {o.blurb ? <span className={hint}>{o.blurb}</span> : null}
          </button>
        );
      })}
    </div>
  );
}

function FieldInput({ id, f, value, problem, onChange, disabled }: { id: string; f: CloudField; value: string; problem?: string; onChange: (v: string) => void; disabled: boolean }) {
  return (
    <div className="grid gap-1.5">
      <Label htmlFor={id} className="flex items-center gap-1.5">
        {f.kind === "reference" ? <KeyRoundIcon className="size-3.5 text-muted-foreground" aria-hidden /> : null}
        {f.label}
        {f.required ? <span className="text-xs font-normal text-muted-foreground">required</span> : null}
      </Label>
      <Input id={id} value={value} placeholder={f.placeholder} onChange={(e) => onChange(e.target.value)} disabled={disabled} spellCheck={false} autoCapitalize="off" autoComplete="off" aria-invalid={!!problem || undefined} className="font-mono text-[13px]" />
      <p className={cn(hint, problem && "text-bad")}>{problem || f.help}</p>
    </div>
  );
}

function Command({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);
  if (!text) return null;
  return (
    <div className="grid gap-1.5">
      <p className="text-sm font-medium">Sign in on this machine, as the user PULSE runs as</p>
      <div className="flex items-start gap-2 rounded-lg bg-muted/50 p-2.5">
        <code className="min-w-0 flex-1 font-mono text-xs leading-relaxed break-all">{text}</code>
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              type="button"
              size="icon-xs"
              variant="ghost"
              aria-label={copied ? "Copied" : "Copy the command"}
              onClick={() => {
                void navigator.clipboard?.writeText(text).then(() => {
                  setCopied(true);
                  setTimeout(() => setCopied(false), 1500);
                });
              }}
            >
              {copied ? <CheckIcon /> : <CopyIcon />}
            </Button>
          </TooltipTrigger>
          <TooltipContent>{copied ? "Copied" : "Copy the command"}</TooltipContent>
        </Tooltip>
      </div>
      <p className={hint}>The secret is read from where it lives as the command runs; it is never shown here or saved by PULSE.</p>
    </div>
  );
}

// "Where machines come from": the cloud, how this machine signs in to it and
// what that cloud needs, under the Machines section of Settings.
export function CloudSetup({ canEdit }: { canEdit: boolean }) {
  const q = useApi<CloudsPayload>("clouds", { refresh: 60000 });
  const [draft, setDraft] = useDraft(q.data);
  const [busy, setBusy] = useState(false);
  const p = q.data;
  const pending = useMemo(() => (p && draft ? changes(p, draft).filter(([n]) => n.startsWith("compute.")) : []), [p, draft]);
  if (!p || !draft) return q.error ? <p className={cn(hint, "px-4 py-4 sm:px-5")}>The cloud list could not be read: {q.error}</p> : null;
  if (!p.available) return null;
  const cloud = cloudOf(p, draft.provider);
  const bad = problems(p, draft);
  const blocked = Object.keys(bad).some((k) => k.startsWith("compute."));
  const set = (name: string, v: string) => setDraft({ ...draft, values: { ...draft.values, [name]: v } });
  const off = !canEdit || busy;
  const missing = cloud ? missingFields(cloud, draft) : [];
  return (
    <form
      className="grid gap-4 px-4 py-4 sm:px-5"
      aria-label="Where machines come from"
      onSubmit={(e) => {
        e.preventDefault();
        if (!pending.length || blocked) return;
        setBusy(true);
        void save(pending, (cloud?.label ?? "Cloud") + " saved.").finally(() => setBusy(false));
      }}
    >
      <div className="grid gap-0.5">
        <h3 className="flex items-center gap-1.5 text-sm font-semibold">
          <CloudIcon className="size-4 text-muted-foreground" aria-hidden />
          Where machines come from
        </h3>
        <p className={hint}>Pick a cloud and fill in what it asks. Sign-ins stay with the cloud&apos;s own tool on this machine; PULSE keeps names, ids and where a secret lives, never the secret.</p>
      </div>
      <Choice name="Cloud" options={p.clouds} value={draft.provider} onPick={(id) => setDraft({ ...draft, provider: id })} disabled={off} />
      {cloud ? (
        <>
          {cloud.auth.length > 1 ? (
            <div className="grid gap-1.5">
              <p className="text-sm font-medium">How this machine signs in to {cloud.label}</p>
              <Choice name={"Sign-in for " + cloud.label} options={cloud.auth} value={authMode(cloud, draft.auth[cloud.id] ?? "")?.id ?? ""} onPick={(id) => setDraft({ ...draft, auth: { ...draft.auth, [cloud.id]: id } })} disabled={off} />
            </div>
          ) : (
            <p className={hint}>{cloud.auth[0]?.blurb}</p>
          )}
          <div className="grid gap-3 sm:grid-cols-2">
            {visibleFields(cloud, draft).map((f) => (
              <FieldInput key={f.setting} id={"cloud-" + f.setting} f={f} value={draft.values[f.setting] ?? ""} problem={bad[f.setting]} onChange={(v) => set(f.setting, v)} disabled={off} />
            ))}
          </div>
          <Command text={signInCommand(cloud, draft)} />
        </>
      ) : null}
      <Notice tone={missing.length ? "warn" : "ok"}>{readiness(p, draft)}</Notice>
      {canEdit ? (
        <div className="flex flex-wrap items-center gap-2">
          <Button type="submit" disabled={!pending.length || blocked || busy}>
            {busy ? <Loader2Icon className="animate-spin" /> : null}
            {pending.length ? "Save " + pending.length + (pending.length === 1 ? " change" : " changes") : "Saved"}
          </Button>
          {pending.length ? (
            <Button type="button" variant="ghost" disabled={busy} onClick={() => setDraft(initialDraft(p))}>
              Undo
            </Button>
          ) : null}
        </div>
      ) : null}
    </form>
  );
}

// "Where backups go": the same registry shape - a target names its parts and
// the cloud whose sign-in it uses - writing backup.to.
export function BackupSetup({ canEdit }: { canEdit: boolean }) {
  const q = useApi<CloudsPayload>("clouds", { refresh: 60000 });
  const [draft, setDraft] = useDraft(q.data);
  const [busy, setBusy] = useState(false);
  const p = q.data;
  const pending = useMemo(() => (p && draft ? changes(p, draft).filter(([n]) => n.startsWith("backup.")) : []), [p, draft]);
  if (!p || !draft || !p.available) return null;
  const t = p.backup.targets.find((x) => x.id === draft.backupTarget);
  const bad = problems(p, draft);
  const blocked = Object.keys(bad).some((k) => k.startsWith("backup"));
  const parts = draft.backupParts[draft.backupTarget] ?? {};
  const setPart = (slot: string, v: string) => setDraft({ ...draft, backupParts: { ...draft.backupParts, [draft.backupTarget]: { ...parts, [slot]: v } } });
  const off = !canEdit || busy;
  const signIn = t?.cloud ? cloudOf(p, t.cloud) : undefined;
  const options = [{ id: "", label: "No backups", blurb: "Nothing leaves this machine." }, ...p.backup.targets];
  return (
    <form
      className="grid gap-4 px-4 py-4 sm:px-5"
      aria-label="Where backups go"
      onSubmit={(e) => {
        e.preventDefault();
        if (!pending.length || blocked) return;
        setBusy(true);
        void save(pending, "Backups saved.").finally(() => setBusy(false));
      }}
    >
      <div className="grid gap-0.5">
        <h3 className="flex items-center gap-1.5 text-sm font-semibold">
          <DatabaseBackupIcon className="size-4 text-muted-foreground" aria-hidden />
          Where backups go
        </h3>
        <p className={hint}>A checked, compressed copy of the PULSE database, off this machine.</p>
      </div>
      <Choice name="Backup destination" options={options} value={draft.backupTarget} onPick={(id) => setDraft({ ...draft, backupTarget: id })} disabled={off} />
      {t ? (
        <div className="grid gap-3 sm:grid-cols-2">
          {t.parts.map((f) => (
            <FieldInput key={f.setting} id={"backup-" + f.setting} f={f} value={parts[f.setting] ?? ""} problem={bad["backup:" + f.setting]} onChange={(v) => setPart(f.setting, v)} disabled={off} />
          ))}
          {t.fields.map((f) => (
            <FieldInput key={f.setting} id={"backup-" + f.setting} f={f} value={draft.values[f.setting] ?? ""} problem={bad[f.setting]} onChange={(v) => setDraft({ ...draft, values: { ...draft.values, [f.setting]: v } })} disabled={off} />
          ))}
        </div>
      ) : null}
      {signIn ? <p className={hint}>Signs in the way {signIn.label} is set up under Machines: {authMode(signIn, draft.auth[signIn.id] ?? "")?.label.toLowerCase()}.</p> : null}
      {t ? (
        <p className={hint}>
          Saved as <code className="font-mono text-xs text-foreground">{backupTo(p, draft) || "…"}</code>
        </p>
      ) : null}
      {canEdit ? (
        <div className="flex flex-wrap items-center gap-2">
          <Button type="submit" disabled={!pending.length || blocked || busy}>
            {busy ? <Loader2Icon className="animate-spin" /> : null}
            {pending.length ? "Save backups" : "Saved"}
          </Button>
          {pending.length ? (
            <Button type="button" variant="ghost" disabled={busy} onClick={() => setDraft(initialDraft(p))}>
              Undo
            </Button>
          ) : null}
        </div>
      ) : null}
    </form>
  );
}
