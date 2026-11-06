"use client";

import Link from "next/link";
import { useEffect, useState } from "react";
import { ArrowRightIcon, CheckIcon, LogOutIcon, RefreshCwIcon, RotateCcwIcon } from "lucide-react";
import { toast } from "sonner";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { Disclosure, ErrorNotice, Notice } from "@/components/pulse/panel";
import { BrandLogo, LogoTile } from "@/components/pulse/logo";
import { Mono, ToneBadge } from "@/components/pulse/who";
import { EMPTY_CHOICE, SettingKey } from "@/components/settings/setting-row";
import { connectedWords, ProviderWizard } from "@/components/settings/provider-wizard";
import { contextChip, modelFamily } from "@/components/settings/settings-state";
import { errorText, writes, type HarnessModel, type HarnessModelsResponse, type HarnessPrompt, type HarnessProviderInfo, type HarnessStatus, type Runner, type Setting } from "@/lib/api";
import { fmtDate, num, rel } from "@/lib/format";
import { logoFor } from "@/lib/logos";
import { hrefs } from "@/lib/nav";
import { CONNECT_PROVIDER_EVENT, providerInfo } from "@/lib/providers";
import { settingHelp, settingLabel } from "@/lib/setting-labels";
import { invalidate, useApi, useNow } from "@/lib/use-api";
import { cn } from "@/lib/utils";

// The Harness section of Settings (the runner points people here as
// "Settings -> Harness"): sign in to the model provider, pick the models,
// cap one run, and - folded away - the system prompt, the tool switches and
// the transcript sizes. These controls save as soon as they change.

export const HARNESS_SETTING_KEYS = ["provider", "harness.max_steps", "harness.task_max_steps", "harness.step_timeout", "harness.output_limit", "harness.max_cost", "harness.keep_observations", "harness.context_window", "harness.compact_at", "harness.trim_keep_recent", "harness.trim_over", "harness.reasoning", "harness.system_prompt", "harness.instructions", "harness.mcp_tools", "harness.skills", "harness.hooks", "harness.subagents", "harness.memory", "harness.tasks", "harness.teach", "harness.web_search", "harness.fetch_allow", "harness.allow_tools", "harness.deny_tools", "worker_model", "light_model", "measure_model", "ask.model", "worker_reasoning", "light_reasoning", "measure_reasoning"];
const SAME_MODEL = "__same__";

// How the chosen provider signs in: who stands behind its device code
// (GitHub for Copilot and GitHub Models, Microsoft for Azure OpenAI) and
// what its pasted secret is called. The hub says which flows a provider
// offers; this only picks the words.
interface SignInWords {
  idp: string;
  tokenLabel: string;
  tokenSummary: string;
  placeholder: string;
  modelsHint: string;
}

export function signInWords(p: HarnessProviderInfo | undefined): SignInWords {
  const name = p?.name ?? "copilot";
  const github = name === "copilot" || name === "github-models";
  const idp = github ? "GitHub" : name === "azure-openai" ? "Microsoft" : p?.label || "the provider";
  const tokenLabel = github ? "GitHub token" : "API key";
  const placeholder = name === "copilot" ? "gho_…" : name === "github-models" ? "github_pat_…" : name === "azure-openai" ? "key or bearer token" : "sk-…";
  const device = p ? p.device_login : true;
  return {
    idp,
    tokenLabel,
    tokenSummary: github ? "Use a GitHub token instead" : "Use an API key instead",
    placeholder,
    modelsHint: device ? "Sign in with " + idp : "Paste " + (github ? "a GitHub token" : "an API key"),
  };
}

function setting(rows: Setting[] | undefined, name: string): Setting | undefined {
  return rows?.find((s) => s.name === name);
}

function modelLabel(m: HarnessModel) {
  return m.name || m.id;
}

// The 16 px brand mark before a model's name: the vendor's when known, else
// the one its id names (gpt-, claude-, gemini-...); nothing when neither matches.
function ModelLogo({ m }: { m: HarnessModel }) {
  const id = logoFor(m.vendor) ?? logoFor(m.id);
  return id ? <BrandLogo id={id} size={16} className="text-muted-foreground" /> : null;
}

function saveSetting(name: string, value: string) {
  return writes.setSetting(name, value).then(() => {
    invalidate("harness");
    invalidate("settings");
  });
}

function sourceText(source: string): { label: string; borrowed: boolean } {
  const s = source.trim().toLowerCase();
  if (s === "device login") return { label: "Signed in here", borrowed: false };
  if (s === "token login" || s === "token") return { label: "With a pasted key", borrowed: false };
  if (s === "anthropic sign-in") return { label: "Signed in through the Anthropic CLI", borrowed: false };
  if (s === "no key") return { label: "No key needed", borrowed: false };
  if (s === "environment") return { label: "Borrowed from this machine's environment", borrowed: true };
  if (s === "editor login") return { label: "Borrowed from your editor", borrowed: true };
  return { label: source || "Unknown", borrowed: false };
}

// One block of the card: a small heading, a one-line note, then its controls.
function Block({ title, note, children, right, className }: { title: React.ReactNode; note?: React.ReactNode; children: React.ReactNode; right?: React.ReactNode; className?: string }) {
  return (
    <div className={cn("grid gap-3 border-t px-4 py-4 first:border-t-0 sm:px-5", className)}>
      <div className="flex min-w-0 flex-wrap items-start justify-between gap-x-4 gap-y-2">
        <div className="min-w-0">
          <h3 className="text-sm font-semibold leading-snug">{title}</h3>
          {note ? <p className="mt-0.5 max-w-2xl text-[13px] leading-relaxed text-muted-foreground anywhere">{note}</p> : null}
        </div>
        {right ? <div className="flex flex-wrap items-center gap-2">{right}</div> : null}
      </div>
      {children}
    </div>
  );
}

function KeyName({ name }: { name: string }) {
  return <SettingKey name={name} />;
}

// The runner's login sentences name this very section ("Settings -> Harness")
// and offer a command. Here they read as plain words; the command goes to
// the "Prefer the terminal?" fold.
const LOGIN_CLI = /[,;]?\s*or run\s+`?(pulse harness login[^`]*?)`?\s*\.?$/i;

export function loginCommand(text: string): string {
  return LOGIN_CLI.exec(text.trim())?.[1]?.trim() ?? "";
}

export function plainLogin(text: string, signedIn: boolean): string {
  const t = text.trim();
  if (!t) return "";
  if (/wait(s|ing)? for .*login/i.test(t)) return signedIn ? "Agents on this machine were waiting for a sign-in. They pick up work again on their next pass." : "Agents on this machine wait until you sign in again.";
  let say = t
    .replace(LOGIN_CLI, "")
    .replace(/\s+in\s+(the console\s+)?\(?Settings\s*->\s*Harness\)?/gi, " below")
    .replace(/\s*\(?Settings\s*->\s*Harness\)?/gi, "")
    .trim();
  say = say.charAt(0).toUpperCase() + say.slice(1);
  return /[.!?]$/.test(say) ? say : say + ".";
}

function ModelSelect({ name, label, note, current, models, disabled }: { name: "worker_model" | "light_model" | "measure_model" | "ask.model"; label: string; note: string; current: string; models: HarnessModel[]; disabled?: boolean }) {
  const [custom, setCustom] = useState(current);
  const ids = new Set(models.map((m) => m.id));
  // The light, measuring and Ask models fall back to another one when empty.
  const sameAs = name === "light_model" || name === "measure_model" ? "Same as the main model" : name === "ask.model" ? "Same as the light model" : null;
  const selectValue = sameAs && !current ? SAME_MODEL : ids.has(current) ? current : "__custom__";
  // The closed picker shows the model's logo and name only; ids and badges stay in the list.
  const known = models.find((m) => m.id === current);
  const shown =
    selectValue === SAME_MODEL && sameAs ? (
      sameAs
    ) : known ? (
      <span className="flex min-w-0 items-center gap-2">
        <ModelLogo m={known} />
        <span className="truncate">{modelLabel(known)}</span>
      </span>
    ) : (
      current || "The default model"
    );
  // Typing an id is the rare path: folded unless the current model isn't in the list, or there is no list.
  const [typing, setTyping] = useState<boolean | null>(null);
  const typeOpen = typing ?? ((!!current && !known) || models.length === 0);
  const save = (value: string) => {
    const v = value === SAME_MODEL ? "" : value;
    const id = toast.loading("Saving the " + label.toLowerCase() + "…");
    saveSetting(name, v)
      .then(() => toast.success(label + " saved.", { id }))
      .catch((e: unknown) => {
        toast.dismiss(id);
        const t = errorText(e);
        toast.error(t.error, { description: [t.why, t.fix].filter(Boolean).join(" ") });
      });
  };
  return (
    <div className="grid min-w-0 content-start gap-1.5">
      <Label id={"model-" + name}>{label}</Label>
      <p className="text-[13px] leading-snug text-muted-foreground">{note}</p>
      <Select value={selectValue} onValueChange={save} disabled={disabled}>
        <SelectTrigger className="w-full" aria-labelledby={"model-" + name}>
          <SelectValue>{shown}</SelectValue>
        </SelectTrigger>
        <SelectContent>
          {sameAs ? <SelectItem value={SAME_MODEL}>{sameAs}</SelectItem> : null}
          {models.map((m) => {
            const family = modelFamily(m.id);
            const context = contextChip(m.context_window);
            return (
              <SelectItem key={m.id} value={m.id}>
                <span className="flex min-w-0 items-center gap-2">
                  <ModelLogo m={m} />
                  <span>{modelLabel(m)}</span>
                  <span className="text-xs text-muted-foreground">{m.id}</span>
                  {family !== "Other" ? <Badge variant="outline">{family}</Badge> : null}
                  {context ? (
                    <Badge variant="outline" className="font-mono" title={num(m.context_window) + " tokens of context"}>
                      {context}
                    </Badge>
                  ) : null}
                  {m.premium > 1 ? <Badge variant="outline" title="Uses more model credits per call">{"×" + m.premium}</Badge> : null}
                  {m.preview ? <Badge variant="outline">Preview</Badge> : null}
                  {m.default ? <Badge variant="secondary">Default</Badge> : null}
                </span>
              </SelectItem>
            );
          })}
          {!ids.has(current) && current ? <SelectItem value="__custom__">{current}</SelectItem> : null}
        </SelectContent>
      </Select>
      <Disclosure summary="Type a model id" open={typeOpen} onToggle={setTyping} contentClassName="grid gap-1.5">
        <Input
          aria-label={label + ": type a model id"}
          value={custom}
          onChange={(e) => setCustom(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") save(custom.trim());
            if (e.key === "Escape") setCustom(current);
          }}
          onBlur={() => {
            if (custom.trim() !== current) save(custom.trim());
          }}
          placeholder="For example claude-sonnet-4.5"
          className="w-full font-mono text-[13px]"
          disabled={disabled}
        />
        <KeyName name={name} />
      </Disclosure>
    </div>
  );
}

// An on/off choice setting of the harness (repository instructions, MCP
// tools) as a switch that saves as soon as it flips.
function OnOffSwitch({ s, onSave }: { s: Setting; onSave: (name: string, value: string) => void }) {
  const on = s.value !== "off";
  return (
    <div className="grid gap-1.5">
      <div className="flex min-h-9 flex-wrap items-center gap-x-3 gap-y-1.5">
        <Switch id={"sw-" + s.name} checked={on} onCheckedChange={(v) => onSave(s.name, v ? "on" : "off")} aria-label={settingLabel(s.name)} />
        <label htmlFor={"sw-" + s.name} className="text-sm font-medium text-foreground">
          {settingLabel(s.name)}
        </label>
        <KeyName name={s.name} />
      </div>
      <p className="text-[13px] leading-snug text-muted-foreground">{settingHelp(s.name, s.description)}</p>
    </div>
  );
}

function ChoiceSelect({ s, onSave, labels, note }: { s: Setting; onSave: (name: string, value: string) => void; labels?: Record<string, string>; note?: string }) {
  const choices = s.choices ?? [];
  const value = choices.includes(s.value) ? s.value : s.default;
  return (
    <div className="grid min-w-0 content-start gap-1.5">
      <Label id={"choice-" + s.name}>{settingLabel(s.name)}</Label>
      {note ? <p className="text-[13px] leading-snug text-muted-foreground">{note}</p> : null}
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1.5">
        <Select value={value || EMPTY_CHOICE} onValueChange={(v) => onSave(s.name, v === EMPTY_CHOICE ? "" : v)}>
          <SelectTrigger className={note ? "w-full" : "w-56 max-w-full"} aria-labelledby={"choice-" + s.name}>
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {choices.map((c) => (
              <SelectItem key={c || EMPTY_CHOICE} value={c || EMPTY_CHOICE}>
                {labels?.[c] ?? (c || "Empty")}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        {note ? null : <KeyName name={s.name} />}
      </div>
      {note ? null : <p className="text-[13px] leading-snug text-muted-foreground">{settingHelp(s.name, s.description)}</p>}
    </div>
  );
}

const WEB_SEARCH_LABELS: Record<string, string> = {
  chat: "Sessions only",
  on: "Sessions and jobs",
  off: "Nowhere",
};

const REASONING_LABELS: Record<string, string> = {
  default: "Model default (send nothing)",
  minimal: "Minimal",
  low: "Low",
  medium: "Medium",
  high: "High",
  xhigh: "Extra high",
};

// The per-role efforts add "" for the harness setting above them.
const ROLE_REASONING_LABELS: Record<string, string> = { "": "Same as the reasoning effort setting", ...REASONING_LABELS };
const MEASURE_REASONING_LABELS: Record<string, string> = { ...ROLE_REASONING_LABELS, "": "Same as the main effort" };

function LimitInput({ s }: { s: Setting }) {
  const [draftState, setDraftState] = useState({ name: s.name, value: s.value, draft: s.value });
  const [error, setError] = useState("");
  const draft = draftState.name === s.name && draftState.value === s.value ? draftState.draft : s.value;
  const setDraft = (value: string) => setDraftState({ name: s.name, value: s.value, draft: value });
  // A budget that may be lifted ("count-or-none") takes none, off or unlimited too.
  const noneWord = (s.kind as string) === "count-or-none" && /^(none|off|unlimited)$/i.test(draft.trim());
  const save = () => {
    if (draft === s.value) return;
    if (!noneWord && (s.kind === "number" ? !/^\d+(\.\d+)?$/.test(draft) : !/^\d+$/.test(draft))) {
      setError(s.kind === "number" ? "Enter a number." : (s.kind as string) === "count-or-none" ? "Enter digits, or none." : "Enter digits only.");
      return;
    }
    if (!noneWord && s.min !== null && Number(draft) < s.min) {
      setError("Enter " + s.min + " or more.");
      return;
    }
    setError("");
    saveSetting(s.name, draft)
      .then(() => toast.success(settingLabel(s.name) + " saved."))
      .catch((e: unknown) => {
        const t = errorText(e);
        setError([t.error, t.why, t.fix].filter(Boolean).join(" "));
      });
  };
  return (
    <div className="grid min-w-0 content-start gap-1.5">
      <Label htmlFor={"harness-" + s.name}>{settingLabel(s.name)}</Label>
      <Input
        id={"harness-" + s.name}
        value={draft}
        inputMode={s.kind === "number" ? "decimal" : (s.kind as string) === "count-or-none" ? undefined : "numeric"}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={save}
        onKeyDown={(e) => {
          if (e.key === "Enter") save();
          if (e.key === "Escape") setDraft(s.value);
        }}
        aria-invalid={!!error}
        aria-describedby={"harness-" + s.name + "-help"}
        className="w-32 tnum"
      />
      <p id={"harness-" + s.name + "-help"} className={cn("text-[13px] leading-snug", error ? "text-bad" : "text-muted-foreground")}>
        {error || settingHelp(s.name, s.description)}
      </p>
      <KeyName name={s.name} />
    </div>
  );
}

// A free-text setting (the fetch tool's allowed hosts) that saves on blur
// or Enter, like a limit does.
function TextInput({ s, placeholder }: { s: Setting; placeholder?: string }) {
  const [draftState, setDraftState] = useState({ name: s.name, value: s.value, draft: s.value });
  const [error, setError] = useState("");
  const draft = draftState.name === s.name && draftState.value === s.value ? draftState.draft : s.value;
  const setDraft = (value: string) => setDraftState({ name: s.name, value: s.value, draft: value });
  const save = () => {
    if (draft.trim() === s.value.trim()) return;
    setError("");
    saveSetting(s.name, draft.trim())
      .then(() => toast.success(settingLabel(s.name) + " saved."))
      .catch((e: unknown) => {
        const t = errorText(e);
        setError([t.error, t.why, t.fix].filter(Boolean).join(" "));
      });
  };
  return (
    <div className="grid min-w-0 content-start gap-1.5">
      <Label htmlFor={"harness-" + s.name}>{settingLabel(s.name)}</Label>
      <Input
        id={"harness-" + s.name}
        value={draft}
        placeholder={placeholder}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={save}
        onKeyDown={(e) => {
          if (e.key === "Enter") save();
          if (e.key === "Escape") setDraft(s.value);
        }}
        aria-invalid={!!error}
        aria-describedby={"harness-" + s.name + "-help"}
        className="max-w-xl font-mono text-[13px]"
      />
      <p id={"harness-" + s.name + "-help"} className={cn("text-[13px] leading-snug", error ? "text-bad" : "text-muted-foreground")}>
        {error || settingHelp(s.name, s.description)}
      </p>
      <KeyName name={s.name} />
    </div>
  );
}

function PromptEditor() {
  const prompt = useApi<HarnessPrompt>("harness/prompt", { refresh: 30000 });
  const [draftState, setDraftState] = useState({ marker: "", draft: "" });
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  const d = prompt.data;
  const marker = d ? (d.custom ? "custom:" + d.template : "default:" + d.default) : "";
  const draft = draftState.marker === marker ? draftState.draft : d?.custom ? d.template : d?.default || "";
  const dirty = !!d && draft !== (d.custom ? d.template : d.default);
  const save = (value: string, label: string) => {
    setSaving(true);
    setError("");
    writes
      .setSetting("harness.system_prompt", value)
      .then(() => {
        toast.success(label);
        invalidate("harness");
        invalidate("harness/prompt");
        invalidate("settings");
        return prompt.reload();
      })
      .catch((e: unknown) => {
        const t = errorText(e);
        setError([t.error, t.why, t.fix].filter(Boolean).join(" "));
      })
      .finally(() => setSaving(false));
  };
  return (
    <Disclosure
      variant="row"
      className="border-t px-3 py-2 sm:px-4"
      contentClassName="mt-1 grid gap-3 pb-3"
      summary={<RowSummary title="System prompt" note="The instructions every run starts with. Most people never change them." badge={d ? <ToneBadge tone={d.custom ? "accent" : "neutral"}>{d.custom ? "Your own" : "Built-in"}</ToneBadge> : null} />}
    >
      <>
        {prompt.error && !d ? <ErrorNotice error={prompt.error} onRetry={() => void prompt.reload()} /> : null}
        {d ? (
          <>
            <p className="text-[13px] text-muted-foreground">PULSE fills in the parts in double braces for each run. Leave it as it is to keep the built-in text.</p>
            <Textarea
              aria-label="System prompt template"
              value={draft}
              onChange={(e) => {
                setDraftState({ marker, draft: e.target.value });
                setError("");
              }}
              rows={10}
              spellCheck={false}
              className={cn("min-h-48 font-mono text-[13px] leading-relaxed", !d.custom && !dirty && "bg-muted/40 text-muted-foreground")}
            />
            <div className="flex flex-wrap items-center gap-2">
              <Button type="button" size="sm" onClick={() => save(draft, "System prompt saved.")} disabled={saving || !dirty}>
                {saving ? <RefreshCwIcon className="animate-spin" /> : <CheckIcon />} Save
              </Button>
              <Button type="button" size="sm" variant="outline" onClick={() => save("", "System prompt reset to the built-in text.")} disabled={saving || (!d.custom && !dirty)}>
                <RotateCcwIcon /> Reset to built-in
              </Button>
              {dirty ? (
                <Button type="button" size="sm" variant="ghost" onClick={() => setDraftState({ marker, draft: d.custom ? d.template : d.default })} disabled={saving}>
                  Undo changes
                </Button>
              ) : null}
              <KeyName name="harness.system_prompt" />
            </div>
            {d.error || error ? <Notice tone="bad">{error || d.error}</Notice> : null}
            <Disclosure summary="Preview the prompt as a run sees it">
              <pre className="max-h-80 overflow-auto whitespace-pre-wrap break-words rounded-lg bg-muted/50 p-3 font-mono text-xs leading-relaxed">{d.rendered || "(empty)"}</pre>
            </Disclosure>
          </>
        ) : !prompt.error ? (
          <p className="text-sm text-muted-foreground">Loading the prompt…</p>
        ) : null}
      </>
    </Disclosure>
  );
}

// The title and note of a folded row inside the card, as the summary of a Disclosure.
function RowSummary({ title, note, badge }: { title: string; note: string; badge?: React.ReactNode }) {
  return (
    <span className="block min-w-0">
      <span className="flex flex-wrap items-center gap-x-2 gap-y-1">
        <span className="text-sm font-semibold leading-snug">{title}</span>
        {badge}
      </span>
      <span className="mt-0.5 block text-sm font-normal leading-relaxed text-muted-foreground">{note}</span>
    </span>
  );
}

// The provider in use, how it is signed in, and the one way to change
// either: the Connect a provider wizard. `?connect=1` on the Settings URL
// opens it (the sidebar's "Connect a model" and Home link there), and
// `?connect=<name>` opens it at that provider's Connect step.
function ProviderBlock({ h, runner, expired, commands, onReload }: { h: HarnessStatus; runner?: Runner; expired: boolean; commands: string[]; onReload: () => void }) {
  const [wizard, setWizard] = useState<{ open: boolean; start?: string }>({ open: false });
  useEffect(() => {
    const ask = new URLSearchParams(window.location.search).get("connect");
    if (!ask) return;
    const id = window.setTimeout(() => {
      // Opened once; a reload shouldn't open it again.
      const url = new URL(window.location.href);
      url.searchParams.delete("connect");
      window.history.replaceState(null, "", url.pathname + url.search + url.hash);
      setWizard({ open: true, start: ask === "1" || ask === "true" ? undefined : ask });
    }, 0);
    return () => window.clearTimeout(id);
  }, []);
  useEffect(() => {
    const open = () => setWizard({ open: true });
    window.addEventListener(CONNECT_PROVIDER_EVENT, open);
    return () => window.removeEventListener(CONNECT_PROVIDER_EVENT, open);
  }, []);
  const p = h.provider;
  const info = providerInfo(p.name, p.label);
  const login = p.login;
  const signedIn = !!login?.logged_in && !expired;
  const [leaving, setLeaving] = useState(false);
  const signOut = () => {
    setLeaving(true);
    writes
      .harnessLogout()
      .then((r) => {
        toast.success(r.login.logged_in ? "Still connected: " + plainLogin(r.login.detail || "a key from the environment is still in use", true) : "Signed out of " + info.title + ".");
        invalidate("harness");
        invalidate("harness/models");
        onReload();
      })
      .catch((e: unknown) => {
        const t = errorText(e);
        toast.error(t.error, { description: [t.why, t.fix].filter(Boolean).join(" ") });
      })
      .finally(() => setLeaving(false));
  };
  return (
    <Block title="Model provider" note="Where agents get their models. Keys and sign-ins stay on this machine.">
      <div className={cn("grid gap-4 rounded-lg border p-4", signedIn ? "bg-card" : "border-warn/40 bg-warn/5")}>
        <div className="flex min-w-0 flex-wrap items-center gap-x-4 gap-y-3">
          <LogoTile id={logoFor(p.name)} name={info.title} size={40} color="brand" title={info.title} />
          <div className="min-w-0 flex-1 basis-56">
            <p className="flex flex-wrap items-center gap-x-2 gap-y-1">
              <span className="text-sm font-semibold">{info.title}</span>
              <ToneBadge tone={signedIn ? "ok" : "warn"} dot>
                {signedIn ? "Connected" : expired ? "Sign-in expired" : "Not connected"}
              </ToneBadge>
            </p>
            <p className="mt-0.5 text-[13px] text-muted-foreground anywhere">
              {signedIn && login?.user?.startsWith("…") ? (
                <>
                  {connectedWords(login)}
                  {login.expires_at ? <span title={fmtDate(login.expires_at)}>, renews {rel(login.expires_at)}</span> : null}.
                </>
              ) : signedIn ? (
                <>
                  {login?.user ? (
                    <>
                      As <b className="font-semibold text-foreground">{login.user}</b>,{" "}
                    </>
                  ) : null}
                  {((l) => l.charAt(0).toLowerCase() + l.slice(1))(sourceText(login?.source || "").label)}
                  {login?.plan ? ", " + login.plan + " plan" : ""}
                  {login?.expires_at ? (
                    <span title={fmtDate(login.expires_at)}>, renews {rel(login.expires_at)}</span>
                  ) : null}
                  .
                </>
              ) : expired ? (
                <>
                  The sign-in{login?.user ? <> as <b className="font-semibold text-foreground">{login.user}</b></> : null} expired{login?.expires_at ? <span title={fmtDate(login.expires_at)}> {rel(login.expires_at)}</span> : null}. Agents wait until you sign in again.
                </>
              ) : (
                "Agents can't run until it's connected."
              )}
            </p>
          </div>
          <div className="flex flex-wrap items-center gap-2">
            {signedIn ? (
              <>
                <Button size="sm" variant="outline" onClick={() => setWizard({ open: true })}>
                  Change provider
                </Button>
                {h.provider.token_login || h.provider.device_login ? (
                  <Button size="sm" variant="ghost" onClick={signOut} disabled={leaving}>
                    <LogOutIcon /> Sign out
                  </Button>
                ) : null}
              </>
            ) : (
              <>
                <Button size="sm" onClick={() => setWizard({ open: true, start: p.name })}>
                  {expired ? "Sign in again" : "Connect " + info.title}
                </Button>
                <Button size="sm" variant="ghost" onClick={() => setWizard({ open: true })}>
                  Choose another provider
                </Button>
              </>
            )}
          </div>
        </div>
        {!signedIn && login?.detail ? <p className="text-[13px] text-muted-foreground anywhere">{plainLogin(login.detail, false)}</p> : null}
        {sourceText(login?.source || "").borrowed && signedIn ? <p className="text-[13px] text-muted-foreground">A borrowed sign-in isn&apos;t shared with your other machines.</p> : null}
      </div>
      {/* While signed out the card already says agents wait; the runner's hold adds news only once the sign-in is back. */}
      {runner?.hold && signedIn ? <Notice tone="info">{plainLogin(runner.hold, signedIn)}</Notice> : null}
      {commands.length ? (
        <Disclosure summary="Prefer the terminal?">
          <p className="text-[13px] text-muted-foreground anywhere">
            Sign in from a terminal on this machine:{" "}
            {commands.map((c, i) => (
              <span key={c}>
                {i ? " or " : null}
                <Mono>{c}</Mono>
              </span>
            ))}
          </p>
        </Disclosure>
      ) : null}
      <ProviderWizard open={wizard.open} start={wizard.start} h={h} onOpenChange={(open) => setWizard((w) => ({ ...w, open }))} />
    </Block>
  );
}

export function HarnessSettingsCard({ className }: { className?: string }) {
  const h = useApi<HarnessStatus>("harness", { refresh: 15000 });
  const runner = useApi<Runner>("runner", { refresh: 15000 });
  const models = useApi<HarnessModelsResponse>("harness/models", { refresh: 60000 });
  const d = h.data;
  const rows = d?.settings;
  const budgets = ["harness.max_steps", "harness.task_max_steps", "minutes_per_job", "harness.max_cost"].map((k) => setting(rows, k)).filter((s): s is Setting => !!s);
  const limits = ["harness.step_timeout", "harness.output_limit", "harness.keep_observations", "harness.context_window", "harness.compact_at", "harness.trim_keep_recent", "harness.trim_over"].map((k) => setting(rows, k)).filter((s): s is Setting => !!s);
  const toggles = ["harness.instructions", "harness.mcp_tools", "harness.skills", "harness.hooks", "harness.subagents", "harness.memory", "harness.tasks", "harness.teach"].map((k) => setting(rows, k)).filter((s): s is Setting => !!s);
  const fetchAllow = setting(rows, "harness.fetch_allow");
  const webSearch = setting(rows, "harness.web_search");
  const allowTools = setting(rows, "harness.allow_tools");
  const denyTools = setting(rows, "harness.deny_tools");
  const reasoning = setting(rows, "harness.reasoning");
  const efforts = (["worker_reasoning", "light_reasoning", "measure_reasoning"] as const).map((k) => setting(rows, k)).filter((s): s is Setting => !!s);
  const words = signInWords(d?.provider);
  const now = useNow();
  const login = d?.provider.login;
  const signedIn = login?.logged_in === true;
  // An expired sign-in is signed in on paper only: offer to sign in again.
  const expired = signedIn && !!login?.expires_at && new Date(login.expires_at).getTime() < now;
  const commands = [...new Set([loginCommand(login?.detail || ""), loginCommand(runner.data?.hold || "")].filter(Boolean))];
  const currentModels = models.data?.models ?? [];
  const modelError = models.error && !models.data ? models.error : "";

  const saveChoice = (name: string, value: string) => {
    const id = toast.loading("Saving " + settingLabel(name).toLowerCase() + "…");
    saveSetting(name, value)
      .then(() => toast.success(settingLabel(name) + " saved.", { id }))
      .catch((e: unknown) => {
        toast.dismiss(id);
        const t = errorText(e);
        toast.error(t.error, { description: [t.why, t.fix].filter(Boolean).join(" ") });
      });
  };

  return (
    <div className={cn("min-w-0 overflow-hidden rounded-lg border bg-card", className)}>
      {h.error && !d ? (
        <div className="p-4 sm:p-5">
          <ErrorNotice error={h.error} onRetry={() => void h.reload()} />
        </div>
      ) : null}
      {!d && !h.error ? <p className="px-4 py-5 text-sm text-muted-foreground sm:px-5">Loading sign-in and models…</p> : null}
      {d ? (
        <>
          <ProviderBlock h={d} runner={runner.data} expired={expired} commands={commands} onReload={() => void h.reload()} />

          <Block title="Models" note="Which models agents use. A change saves as soon as you pick it.">
            {modelError && (!signedIn || expired) ? (
              <p className="text-[13px] text-muted-foreground">Connect {providerInfo(d.provider.name, d.provider.label).title} above to pick from its model list. Until then you can type a model id.</p>
            ) : modelError ? (
              <Notice tone={modelError.includes("not signed in") ? "warn" : "bad"}>
                <span className="grid gap-1">
                  <span>The model list didn&apos;t load: {modelError}</span>
                  <span>
                    Main model now: <Mono>{d.models.worker || "the default"}</Mono>. Light model now: <Mono>{d.models.light || "same as the main model"}</Mono>.
                  </span>
                  <span>
                    {words.modelsHint} to see the {d.provider.label} models, or type the exact model id under Type a model id.
                  </span>
                </span>
              </Notice>
            ) : null}
            <div className="grid gap-5 md:grid-cols-2 xl:grid-cols-4">
              <ModelSelect name="worker_model" label="Main model" note="For the coding work itself." current={d.models.worker} models={currentModels} disabled={false} />
              <ModelSelect name="light_model" label="Light model" note="For smaller jobs like planning, answering and voting." current={d.models.light} models={currentModels} disabled={false} />
              <ModelSelect name="measure_model" label="Measuring model" note="For measuring baselines, experiments and merges; a cheap, steady model keeps the loop honest and affordable." current={d.models.measure ?? ""} models={currentModels} disabled={false} />
              <ModelSelect name="ask.model" label="Ask model" note="For your questions in Ask; a fast model keeps answers quick." current={d.models.ask ?? ""} models={currentModels} disabled={false} />
            </div>
            {efforts.length ? (
              <div className="mt-5 grid gap-5 md:grid-cols-2 xl:grid-cols-4">
                {efforts.map((s) => (
                  <ChoiceSelect
                    key={s.name}
                    s={s}
                    onSave={saveChoice}
                    labels={s.name === "measure_reasoning" ? MEASURE_REASONING_LABELS : ROLE_REASONING_LABELS}
                    note={s.name === "worker_reasoning" ? "How hard the main model thinks on the coding work." : s.name === "light_reasoning" ? "How hard the light model thinks on the smaller jobs." : "How hard the measuring model thinks; a change measures every baseline again."}
                  />
                ))}
              </div>
            ) : null}
          </Block>

          <Block title="Limits per run" note="A run stops when it reaches any of these. Each one saves when you leave the field.">
            <div className="grid gap-5 sm:grid-cols-3">{budgets.map((s) => <LimitInput key={s.name} s={s} />)}</div>
          </Block>

          <PromptEditor />

          <Disclosure
            variant="row"
            className="border-t px-3 py-2 sm:px-4"
            contentClassName="mt-1 grid gap-5 pb-3"
            summary={<RowSummary title="Tools and transcript size" note="What an agent may use while it works, and how much command output a run keeps." />}
          >
            <>
              {toggles.length ? (
                <div className="grid gap-4 sm:grid-cols-2">
                  {toggles.map((s) => (
                    <OnOffSwitch key={s.name} s={s} onSave={saveChoice} />
                  ))}
                </div>
              ) : null}
              {fetchAllow ? <TextInput s={fetchAllow} placeholder="docs.rs, *.rust-lang.org" /> : null}
              {webSearch ? <ChoiceSelect s={webSearch} onSave={saveChoice} labels={WEB_SEARCH_LABELS} /> : null}
              {denyTools ? <TextInput s={denyTools} placeholder="bash(rm -rf *), read(.env*), write(*.lock)" /> : null}
              {allowTools ? <TextInput s={allowTools} placeholder="bash(git *), bash(cargo *), edit(src/*)" /> : null}
              {reasoning ? <ChoiceSelect s={reasoning} onSave={saveChoice} labels={REASONING_LABELS} /> : null}
              <div className="grid gap-5 sm:grid-cols-2 lg:grid-cols-4">{limits.map((s) => <LimitInput key={s.name} s={s} />)}</div>
            </>
          </Disclosure>

          <div className="flex flex-wrap items-center justify-between gap-3 border-t bg-muted/40 px-4 py-3 sm:px-5">
            <p className="min-w-0 text-[13px] leading-relaxed text-muted-foreground">
              Today: {num(d.sessions.today)} {d.sessions.today === 1 ? "run" : "runs"}, {num(d.sessions.submitted_today)} finished, {num(d.sessions.failed_today)} failed. {num(d.sessions.running)} running now. {num(d.sessions.cost_today)} model credits used.
            </p>
            <Button asChild size="sm" variant="outline">
              <Link href={hrefs.harness()}>
                Open Runs <ArrowRightIcon />
              </Link>
            </Button>
          </div>
        </>
      ) : null}
    </div>
  );
}

export function isHarnessSetting(name: string): boolean {
  return HARNESS_SETTING_KEYS.includes(name);
}
