"use client";

import Link from "next/link";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ArrowLeftIcon, ArrowRightIcon, CheckIcon, CircleCheckIcon, ClipboardIcon, CloudIcon, CpuIcon, ExternalLinkIcon, KeyRoundIcon, LaptopIcon, RefreshCwIcon, SearchIcon, ServerCogIcon, ShieldCheckIcon, UserRoundCheckIcon } from "lucide-react";
import { toast } from "sonner";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { Stepper, useWizardLock, WizardDialog } from "@/components/pulse/stepper";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { Disclosure, Notice } from "@/components/pulse/panel";
import { Mono } from "@/components/pulse/who";
import { errorText, harnessModelsPath, harnessProviderPath, writes, type HarnessDeviceFlow, type HarnessLogin, type HarnessModel, type HarnessModelsResponse, type HarnessProviderInfo, type HarnessStatus } from "@/lib/api";
import { num } from "@/lib/format";
import { hrefs } from "@/lib/nav";
import { groupProviders, keyProblem, providerInfo, suggestLightModel, suggestModel, type ProviderInfo, type ProviderKind } from "@/lib/providers";
import { invalidate, useApi, useNow } from "@/lib/use-api";
import { cn } from "@/lib/utils";

// Connect a model provider, as four short steps instead of one long form:
// pick the provider, connect it (a browser sign-in, a pasted key, or the
// address of a local server), choose the models, then review and switch.
// Signing in happens before the switch (the hub takes the provider's name
// on every sign-in call), so a provider half set up never stops the agents
// that are running on the current one. The models step matters: switching
// provider leaves model ids that only the old provider knows, which would
// fail every run.

type Step = "pick" | "connect" | "models" | "review" | "done";
const STEPS: { id: Exclude<Step, "done">; label: string }[] = [
  { id: "pick", label: "Provider" },
  { id: "connect", label: "Connect" },
  { id: "models", label: "Models" },
  { id: "review", label: "Review" },
];
const SAME = "__same__";

const KIND_ICON: Record<ProviderKind, React.ComponentType<{ className?: string }>> = {
  account: UserRoundCheckIcon,
  key: KeyRoundIcon,
  local: LaptopIcon,
  custom: ServerCogIcon,
};

function settingValue(h: HarnessStatus, name: string): string | undefined {
  return h.settings.find((s) => s.name === name)?.value;
}

// A sign-in past its expiry is signed in on paper only: it needs a new one.
function expiredLogin(login: Pick<HarnessLogin, "logged_in" | "expires_at"> | undefined): boolean {
  return !!login?.logged_in && !!login.expires_at && new Date(login.expires_at).getTime() < Date.now();
}

function errorLine(e: unknown): string {
  const t = errorText(e);
  return [t.error, t.why, t.fix].filter(Boolean).join(" ");
}

function sourceWords(source: string): string {
  const s = source.trim().toLowerCase();
  if (s === "device login") return "signed in here";
  if (s === "token login" || s === "token") return "with a pasted key";
  if (s === "anthropic sign-in") return "through the Anthropic CLI";
  if (s === "environment") return "with a key from this machine's environment";
  if (s === "editor login") return "borrowed from your editor";
  if (s === "no key") return "no key needed";
  return source;
}

// Step 1: the providers as cards in their groups, the one in use marked.
function PickStep({ h, choice, onChoose, onNext }: { h: HarnessStatus; choice: string; onChoose: (name: string) => void; onNext: () => void }) {
  const [q, setQ] = useState("");
  const groups = useMemo(() => groupProviders(h.providers, q), [h.providers, q]);
  return (
    <div className="grid grid-cols-1 gap-5">
      <div className="relative">
        <SearchIcon aria-hidden className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground" />
        <Input value={q} onChange={(e) => setQ(e.target.value)} placeholder="Search providers" aria-label="Search providers" className="h-10 pl-9" type="search" />
      </div>
      {groups.length === 0 ? <p className="text-sm text-muted-foreground">No provider matches &ldquo;{q.trim()}&rdquo;.</p> : null}
      {groups.map((g) => {
        const Icon = KIND_ICON[g.kind];
        return (
          <section key={g.kind} aria-label={g.title} className="grid grid-cols-1 gap-2">
            <div className="flex flex-wrap items-baseline gap-x-2 gap-y-0.5">
              <Icon aria-hidden className="size-4 translate-y-0.5 text-muted-foreground" />
              <h3 className="text-sm font-semibold">{g.title}</h3>
              {g.note ? <p className="min-w-0 text-[13px] text-muted-foreground max-sm:basis-full">{g.note}</p> : null}
            </div>
            <div role="radiogroup" aria-label={g.title} className="grid gap-2 sm:grid-cols-2">
              {g.items.map(({ hub, info }) => {
                const on = choice === hub.name;
                const inUse = h.provider.name === hub.name;
                return (
                  <button
                    key={hub.name}
                    type="button"
                    role="radio"
                    aria-checked={on}
                    onClick={() => onChoose(hub.name)}
                    onDoubleClick={() => {
                      onChoose(hub.name);
                      onNext();
                    }}
                    className={cn(
                      "group grid min-w-0 gap-1 rounded-xl border bg-card p-3 text-left transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
                      on ? "border-primary ring-1 ring-primary" : "hover:border-foreground/25 hover:bg-muted/40",
                    )}
                  >
                    <span className="flex min-w-0 items-center gap-2">
                      <ProviderGlyph name={hub.name} title={info.title} />
                      <span className="min-w-0 flex-1 truncate text-sm font-semibold">{info.title}</span>
                      {inUse ? (
                        <Badge variant="secondary" className="shrink-0">
                          In use
                        </Badge>
                      ) : info.badge ? (
                        <Badge variant="outline" className="shrink-0">
                          {info.badge}
                        </Badge>
                      ) : null}
                      <span aria-hidden className={cn("inline-flex size-4 shrink-0 items-center justify-center rounded-full border", on ? "border-primary bg-primary text-primary-foreground" : "border-input")}>
                        {on ? <CheckIcon className="size-3" /> : null}
                      </span>
                    </span>
                    {info.blurb ? <span className="line-clamp-2 text-[13px] leading-snug text-muted-foreground">{info.blurb}</span> : null}
                  </button>
                );
              })}
            </div>
          </section>
        );
      })}
    </div>
  );
}

// How a provider is connected, as one phrase: a masked key reads as the
// key's ending ("With a pasted key ending in mnop"), an account by name.
export function connectedWords(login: Pick<HarnessLogin, "user" | "source"> | undefined): string {
  const user = login?.user?.trim() ?? "";
  const how = sourceWords(login?.source ?? "");
  if (user.startsWith("…")) return "With a " + (how === "with a key from this machine's environment" ? "key from this machine's environment" : "pasted key") + " ending in " + user.slice(1);
  if (!user) return how ? how.charAt(0).toUpperCase() + how.slice(1) : "Connected";
  return user + (how ? ", " + how : "");
}

// A small square with the provider's initials, in a hue of its own, so the
// cards and the summary card are told apart at a glance without logos.
export function ProviderGlyph({ name, title, className }: { name: string; title: string; className?: string }) {
  let hash = 0;
  for (const c of name) hash = (hash * 31 + c.charCodeAt(0)) % 360;
  const words = title.replace(/\(.*\)/, "").trim().split(/\s+/);
  const initials = (words.length > 1 ? words[0]![0]! + words[1]![0]! : title.slice(0, 2)).toUpperCase();
  return (
    <span
      aria-hidden
      className={cn("inline-flex size-7 shrink-0 items-center justify-center rounded-lg bg-[oklch(0.93_0.05_var(--glyph-h))] text-[11px] font-bold tracking-tight text-[oklch(0.38_0.1_var(--glyph-h))] dark:bg-[oklch(0.34_0.06_var(--glyph-h))] dark:text-[oklch(0.88_0.07_var(--glyph-h))]", className)}
      style={{ "--glyph-h": hash } as React.CSSProperties}
    >
      {initials}
    </span>
  );
}

// A browser sign-in: a device code to type at the provider (GitHub,
// Microsoft), or - with no code - a sign-in Anthropic's CLI opened in a
// browser on the machine PULSE runs on. Polls until done.
function BrowserSignIn({ provider, info, onSignedIn, before }: { provider: string; info: ProviderInfo; onSignedIn: (login: HarnessLogin) => void; before: () => Promise<void> }) {
  const idp = info.idp || info.title;
  const [flow, setFlow] = useState<HarnessDeviceFlow | null>(null);
  const [status, setStatus] = useState<"idle" | "starting" | "pending" | "expired" | "denied">("idle");
  const [delay, setDelay] = useState(5);
  const [polls, setPolls] = useState(0);
  const [error, setError] = useState("");
  const [startedAt, setStartedAt] = useState(0);
  const now = useNow(1000);
  const remaining = flow ? Math.max(0, Math.ceil((startedAt + flow.expires_in * 1000 - now) / 1000)) : 0;
  const codeless = !!flow && !flow.user_code;

  const start = () => {
    setStatus("starting");
    setError("");
    before()
      .then(() => writes.harnessLogin(provider))
      .then((r) => {
        setFlow(r);
        setStartedAt(Date.now());
        setDelay(Math.max(1, r.interval || 5));
        setPolls(0);
        setStatus("pending");
      })
      .catch((e: unknown) => {
        setError(errorLine(e));
        setStatus("idle");
      });
  };

  useEffect(() => {
    if (!flow || status !== "pending") return;
    const id = setTimeout(() => {
      writes
        .harnessLoginPoll(flow.device_code, provider)
        .then((r) => {
          setPolls((n) => n + 1);
          if (r.status === "slow_down") setDelay((n) => n + 5);
          if (r.status === "expired") setStatus("expired");
          if (r.status === "denied") setStatus("denied");
          if (r.status === "complete" && r.login) {
            setFlow(null);
            setStatus("idle");
            onSignedIn(r.login);
          }
        })
        .catch((e: unknown) => {
          setError(errorLine(e));
          setFlow(null);
          setStatus("idle");
        });
    }, delay * 1000);
    return () => clearTimeout(id);
  }, [flow, status, delay, polls, provider, onSignedIn]);

  if (!flow)
    return (
      <div className="grid gap-2">
        <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
          <Button type="button" onClick={start} disabled={status === "starting"}>
            {status === "starting" ? <RefreshCwIcon className="animate-spin" /> : <ShieldCheckIcon />} Sign in with {idp}
          </Button>
          <span className="text-[13px] text-muted-foreground">{info.signInNote ? "Opens a browser window to approve PULSE." : "You'll get a short code to enter on " + idp + "."}</span>
        </div>
        {info.signInNote ? <p className="text-[13px] leading-relaxed text-muted-foreground">{info.signInNote}</p> : null}
        {error ? (
          <Notice tone="bad" className="text-[13px]">
            {error}
          </Notice>
        ) : null}
      </div>
    );

  return (
    <div className="grid gap-3 rounded-xl border bg-muted/40 p-4" aria-live="polite">
      {codeless ? (
        <>
          <p className="text-sm font-medium">Approve PULSE in the browser window that just opened.</p>
          <p className="text-[13px] leading-relaxed text-muted-foreground">
            It opened on the machine PULSE runs on. Pick your organization and workspace, approve, and come back. This page notices by itself.
          </p>
          {flow.verification_uri ? (
            <Button asChild size="sm" variant="outline" className="w-fit">
              <a href={flow.verification_uri} target="_blank" rel="noreferrer">
                Didn&apos;t open? Open the sign-in page <ExternalLinkIcon />
              </a>
            </Button>
          ) : null}
        </>
      ) : (
        <>
          <ol className="grid gap-1 text-sm text-muted-foreground">
            <li>1. Open {idp} and enter this code.</li>
            <li>2. Approve the sign-in. This page notices by itself.</li>
          </ol>
          <div className="flex flex-wrap items-center justify-between gap-3">
            <p className="font-mono text-2xl font-semibold tracking-[0.12em] text-foreground" aria-label={"Code " + flow.user_code.split("").join(" ")}>
              {flow.user_code}
            </p>
            <div className="flex flex-wrap gap-2">
              <Button
                type="button"
                size="sm"
                variant="outline"
                onClick={() =>
                  navigator.clipboard
                    .writeText(flow.user_code)
                    .then(() => toast.success("Code copied."))
                    .catch(() => toast.error("Couldn't copy. Select the code and copy it yourself."))
                }
              >
                <ClipboardIcon /> Copy code
              </Button>
              <Button asChild size="sm">
                <a href={flow.verification_uri} target="_blank" rel="noreferrer">
                  Open {idp} <ExternalLinkIcon />
                </a>
              </Button>
            </div>
          </div>
        </>
      )}
      <p className="flex items-center gap-2 text-[13px] text-muted-foreground">
        {status === "pending" ? <RefreshCwIcon aria-hidden className="size-3.5 animate-spin" /> : null}
        {status === "pending" ? "Waiting for " + idp + "…" + (remaining > 0 && remaining < 120 ? " " + num(remaining) + " seconds left." : "") : status === "expired" ? "The sign-in timed out. Try again." : "The sign-in was turned down on " + idp + "."}
      </p>
      <div className="flex gap-2">
        {status === "expired" || status === "denied" ? (
          <Button size="sm" onClick={start}>
            Try again
          </Button>
        ) : null}
        <Button size="sm" variant="ghost" onClick={() => setFlow(null)}>
          Cancel
        </Button>
      </div>
    </div>
  );
}

// A pasted key: where to get one, a check for keys that are clearly wrong,
// then the hub validates it with the provider before keeping it.
function KeyForm({ provider, info, optional, onSignedIn, before }: { provider: string; info: ProviderInfo; optional?: boolean; onSignedIn: (login: HarnessLogin) => void; before: () => Promise<void> }) {
  const [key, setKey] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const problem = keyProblem(provider, key);
  const github = provider === "copilot" || provider === "github-models";
  const word = github ? "token" : "API key";
  const submit = () => {
    if (!key.trim() || problem) return;
    setBusy(true);
    setError("");
    before()
      .then(() => writes.harnessLoginToken(key.trim(), provider))
      .then((r) => {
        setKey("");
        onSignedIn(r.login);
      })
      .catch((e: unknown) => setError(errorLine(e)))
      .finally(() => setBusy(false));
  };
  return (
    <div className="grid gap-2">
      <div className="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1">
        <Label htmlFor={"key-" + provider}>
          {info.title} {word}
          {optional ? <span className="font-normal text-muted-foreground"> (only if your server asks for one)</span> : null}
        </Label>
        {info.keyUrl ? (
          <a href={info.keyUrl} target="_blank" rel="noreferrer" className="inline-flex items-center gap-1 text-[13px] font-medium text-primary hover:underline">
            Get {github ? "a token" : "a key"} at {info.keySite} <ExternalLinkIcon aria-hidden className="size-3.5" />
          </a>
        ) : null}
      </div>
      <div className="flex flex-wrap gap-2 sm:flex-nowrap">
        <Input
          id={"key-" + provider}
          type="password"
          autoComplete="off"
          spellCheck={false}
          value={key}
          onChange={(e) => {
            setKey(e.target.value);
            setError("");
          }}
          onKeyDown={(e) => e.key === "Enter" && submit()}
          placeholder={info.keyHint || "Paste the key"}
          aria-invalid={!!(problem || error)}
          aria-describedby={"key-" + provider + "-help"}
          className="min-w-0 flex-1 font-mono text-[13px]"
        />
        <Button type="button" onClick={submit} disabled={!key.trim() || !!problem || busy}>
          {busy ? <RefreshCwIcon className="animate-spin" /> : <KeyRoundIcon />} {busy ? "Checking…" : "Connect"}
        </Button>
      </div>
      <p id={"key-" + provider + "-help"} className={cn("text-[13px] leading-relaxed", problem || error ? "text-bad" : "text-muted-foreground")}>
        {problem || error || "PULSE checks the " + word + " with " + info.title + " before keeping it. It stays on this machine and never goes to agents."}
      </p>
    </div>
  );
}

// The address a provider's calls go to: open for local servers, Azure and
// custom servers (they need one), folded for the cloud vendors.
function AddressField({ info, value, onChange, error }: { info: ProviderInfo; value: string; onChange: (v: string) => void; error?: string }) {
  if (!info.baseUrlSetting) return null;
  const field = (
    <div className="grid gap-1.5">
      <Label htmlFor={"addr-" + info.name}>{info.kind === "local" ? "Server address" : info.name === "azure-openai" ? "Resource address" : "Address"}</Label>
      <Input
        id={"addr-" + info.name}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={info.defaultBaseUrl || (info.name === "azure-openai" ? "https://<resource>.openai.azure.com/openai/v1" : "https://<host>/v1")}
        spellCheck={false}
        aria-invalid={!!error}
        className="font-mono text-[13px]"
      />
      <p className={cn("text-[13px]", error ? "text-bad" : "text-muted-foreground")}>
        {error ||
          (info.kind === "local"
            ? "Where the model server listens on this computer. The default works for a standard install."
            : info.needsAddress
              ? "Required. Copy it from the provider's portal."
              : "Leave it empty to use " + info.title + "'s own address. Change it only for a gateway or proxy.")}
      </p>
    </div>
  );
  if (info.needsAddress || info.kind === "local") return field;
  return (
    <Disclosure summary="Use a different address" open={value ? true : undefined}>
      {field}
    </Disclosure>
  );
}

const validUrl = (v: string) => {
  if (!v.trim()) return true;
  try {
    const u = new URL(v.trim());
    return u.protocol === "http:" || u.protocol === "https:";
  } catch {
    return false;
  }
};

// Step 2: connect. Shows what the provider already has on this machine,
// then the ways in that the hub says it offers.
function ConnectStep({ h, provider, address, setAddress, addressError, login, onLogin, customNoKey, setCustomNoKey, reloadLogin }: { h: HarnessStatus; provider: string; address: string; setAddress: (v: string) => void; addressError: string; login: HarnessLogin | undefined; onLogin: (l: HarnessLogin) => void; customNoKey: boolean; setCustomNoKey: (v: boolean) => void; reloadLogin: () => void }) {
  const hub = h.providers.find((p) => p.name === provider);
  const info = providerInfo(provider, hub?.label);
  const device = !!hub?.device_login;
  const token = !!hub?.token_login;
  const local = info.kind === "local" || (provider === "custom" && customNoKey);
  const [way, setWay] = useState<"signin" | "key">(device ? "signin" : "key");
  const stale = expiredLogin(login);
  const connected = !!login?.logged_in && !stale;
  const [again, setAgain] = useState(false);
  const signedIn = useCallback(
    (l: HarnessLogin) => {
      toast.success(l.user ? "Connected as " + l.user + "." : "Connected.");
      invalidate("harness");
      setAgain(false);
      onLogin(l);
    },
    [onLogin],
  );
  const models = useApi<HarnessModelsResponse>(local ? harnessModelsPath(provider) : null, { refresh: false });
  // A sign-in checks the key against the provider's address, so a changed one is saved first.
  const saveAddress = useCallback(() => {
    const key = info.baseUrlSetting;
    if (!key || addressError || address.trim() === (settingValue(h, key) ?? "")) return Promise.resolve();
    return writes.setSetting(key, address.trim()).then(() => invalidate("harness"));
  }, [info.baseUrlSetting, addressError, address, h]);

  return (
    <div className="grid gap-5">
      <div className="flex items-start gap-3">
        <ProviderGlyph name={provider} title={info.title} className="size-10 text-sm" />
        <div className="min-w-0">
          <h3 className="text-[15px] font-semibold">{info.title}</h3>
          {info.blurb ? <p className="text-[13px] leading-relaxed text-muted-foreground">{info.blurb}</p> : null}
        </div>
      </div>

      <AddressField info={info} value={address} onChange={setAddress} error={addressError} />

      {provider === "custom" ? (
        <div className="flex items-start gap-3">
          <Switch id="custom-no-key" checked={customNoKey} onCheckedChange={setCustomNoKey} />
          <label htmlFor="custom-no-key" className="grid gap-0.5 text-sm">
            <span className="font-medium">The server needs no key</span>
            <span className="text-[13px] text-muted-foreground">Like a model server on your own network.</span>
          </label>
        </div>
      ) : null}

      {stale && !local ? (
        <Notice tone="warn">
          The sign-in{login?.user && !login.user.startsWith("…") ? <> as <b className="font-semibold">{login.user}</b></> : null} expired. Sign in again to keep using {info.title}.
        </Notice>
      ) : null}

      {connected && !again && !local ? (
        <div className="flex flex-wrap items-center justify-between gap-3 rounded-xl border border-ok/30 bg-ok/8 px-4 py-3">
          <p className="flex min-w-0 items-center gap-2 text-sm">
            <CircleCheckIcon aria-hidden className="size-4 shrink-0 text-ok" />
            <span className="min-w-0 anywhere">
              {login?.user?.startsWith("…") ? (
                <>Connected. {connectedWords(login)}.</>
              ) : (
                <>
                  Connected{login?.user ? <> as <b className="font-semibold">{login.user}</b></> : null}
                  {login?.source ? <span className="text-muted-foreground">, {sourceWords(login.source)}</span> : null}.
                </>
              )}
            </span>
          </p>
          {device || token ? (
            <Button size="sm" variant="ghost" onClick={() => setAgain(true)}>
              Use another account
            </Button>
          ) : null}
        </div>
      ) : null}

      {local ? (
        <div className="grid gap-3">
          <div className="flex flex-wrap items-center gap-3">
            <Button
              type="button"
              variant="outline"
              onClick={() => {
                void saveAddress().then(() => {
                  void models.reload();
                  reloadLogin();
                });
              }}
              disabled={!!addressError || models.loading}
            >
              {models.loading ? <RefreshCwIcon className="animate-spin" /> : <CpuIcon />} Check the connection
            </Button>
            {models.data ? (
              <span className="flex items-center gap-1.5 text-sm text-ok">
                <CircleCheckIcon aria-hidden className="size-4" /> Reached it: {num(models.data.models.length)} {models.data.models.length === 1 ? "model" : "models"} available.
              </span>
            ) : null}
          </div>
          {models.error && !models.data ? (
            <Notice tone="warn" className="text-[13px]">
              <span className="grid gap-1">
                <span>PULSE couldn&apos;t reach the server: {models.error}</span>
                <span>
                  {info.name === "ollama" ? (
                    <>
                      Start it with <Mono>ollama serve</Mono> and pull a model, for example <Mono>ollama pull qwen3-coder</Mono>.
                    </>
                  ) : info.name === "lmstudio" ? (
                    "Open LM Studio, load a model and turn on its local server."
                  ) : (
                    "Check that the server is running and the address is right."
                  )}
                </span>
              </span>
            </Notice>
          ) : null}
          {token && provider !== "custom" ? (
            <Disclosure summary="My server needs a key">
              <KeyForm provider={provider} info={info} optional onSignedIn={signedIn} before={saveAddress} />
            </Disclosure>
          ) : null}
        </div>
      ) : !connected || again ? (
        <div className="grid gap-4">
          {device && token ? (
            <div role="tablist" aria-label="How to connect" className="inline-flex w-fit rounded-lg bg-muted p-0.5">
              {(
                [
                  ["signin", "Sign in"],
                  ["key", provider === "copilot" || provider === "github-models" ? "Token" : "API key"],
                ] as const
              ).map(([id, label]) => (
                <button
                  key={id}
                  type="button"
                  role="tab"
                  aria-selected={way === id}
                  onClick={() => setWay(id)}
                  className={cn("rounded-md px-3 py-1.5 text-[13px] font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring", way === id ? "bg-card text-foreground shadow-[0_1px_2px_oklch(0.26_0.04_262/.08)]" : "text-muted-foreground hover:text-foreground")}
                >
                  {label}
                </button>
              ))}
            </div>
          ) : null}
          {device && (way === "signin" || !token) ? <BrowserSignIn provider={provider} info={info} onSignedIn={signedIn} before={saveAddress} /> : null}
          {token && (way === "key" || !device) ? <KeyForm provider={provider} info={info} onSignedIn={signedIn} before={saveAddress} /> : null}
          {!device && !token ? <Notice tone="warn">This provider can&apos;t be connected from the console. Use the terminal: <Mono>{"pulse harness login --provider " + provider}</Mono></Notice> : null}
          {provider === "anthropic" ? (
            <Disclosure summary="Can I use my Claude Pro or Max plan?">
              <p className="text-[13px] leading-relaxed text-muted-foreground">
                No. Anthropic lets only its own apps, such as Claude and Claude Code, sign in with a Pro or Max plan, and it turns away other apps that try. Sign in with Anthropic uses your Anthropic Console account instead, through Anthropic&apos;s own command-line tool. It is billed per token, like an API key, and needs no key to copy.
              </p>
            </Disclosure>
          ) : null}
          {provider === "anthropic" && device ? (
            <Disclosure summary="The sign-in needs Anthropic's command-line tool">
              <p className="text-[13px] leading-relaxed text-muted-foreground">
                PULSE asks Anthropic&apos;s CLI (<Mono>ant</Mono>) for short-lived sign-in tokens, so no key is stored here. If it isn&apos;t installed on the machine PULSE runs on, install it there (on a Mac: <Mono>brew install anthropics/tap/ant</Mono>; elsewhere a release from github.com/anthropics/anthropic-cli) and try again, or paste an API key instead. Signing out here doesn&apos;t sign the CLI out, so your other tools keep working.
              </p>
            </Disclosure>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

function ModelPicker({ id, label, note, value, onChange, models, sameLabel }: { id: string; label: string; note: string; value: string; onChange: (v: string) => void; models: HarnessModel[]; sameLabel?: string }) {
  const known = models.find((m) => m.id === value);
  const shown = !value && sameLabel ? sameLabel : known ? known.name || known.id : value || "The provider's default";
  return (
    <div className="grid min-w-0 content-start gap-1.5">
      <Label id={id}>{label}</Label>
      <p className="text-[13px] leading-snug text-muted-foreground">{note}</p>
      <Select value={!value && sameLabel ? SAME : value || "__default__"} onValueChange={(v) => onChange(v === SAME || v === "__default__" ? "" : v)}>
        <SelectTrigger className="w-full" aria-labelledby={id}>
          <SelectValue>{shown}</SelectValue>
        </SelectTrigger>
        <SelectContent>
          {sameLabel ? <SelectItem value={SAME}>{sameLabel}</SelectItem> : <SelectItem value="__default__">The provider&apos;s default</SelectItem>}
          {models.map((m) => (
            <SelectItem key={m.id} value={m.id}>
              <span className="flex min-w-0 items-center gap-1.5">
                <span>{m.name || m.id}</span>
                {m.name && m.name !== m.id ? <span className="text-xs text-muted-foreground">{m.id}</span> : null}
                {m.context_window >= 1_000_000 ? <Badge variant="outline">1M context</Badge> : null}
                {m.preview ? <Badge variant="outline">Preview</Badge> : null}
                {m.default ? <Badge variant="secondary">Default</Badge> : null}
              </span>
            </SelectItem>
          ))}
          {value && !known ? <SelectItem value={value}>{value}</SelectItem> : null}
        </SelectContent>
      </Select>
    </div>
  );
}

// Step 3: the models of the chosen provider for the two roles that matter
// most. The measuring and Ask models stay in Settings.
function ModelsStep({ provider, title, main, setMain, light, setLight, onModels }: { provider: string; title: string; main: string; setMain: (v: string) => void; light: string; setLight: (v: string) => void; onModels: (m: HarnessModel[]) => void }) {
  const q = useApi<HarnessModelsResponse>(harnessModelsPath(provider), { refresh: false });
  const models = useMemo(() => (q.data?.models ?? []).filter((m) => m.chat !== false), [q.data]);
  const [typing, setTyping] = useState(false);
  // A list cached before the sign-in (or the "not signed in" it answered) is stale now.
  const { reload, updatedAt } = q;
  useEffect(() => {
    if (updatedAt && Date.now() - updatedAt > 1500) void reload();
    // Once, when the step opens.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  useEffect(() => {
    if (q.data) onModels(models);
  }, [q.data, models, onModels]);
  if (q.loading && !q.data)
    return (
      <p className="flex items-center gap-2 text-sm text-muted-foreground">
        <RefreshCwIcon aria-hidden className="size-4 animate-spin" /> Loading {title}&apos;s models…
      </p>
    );
  return (
    <div className="grid gap-5">
      {q.error && !q.data ? (
        <Notice tone="warn">
          The model list didn&apos;t load: {q.error}. You can type model ids below, or keep the provider&apos;s default.{" "}
          <button type="button" className="font-medium text-primary hover:underline" onClick={() => void q.reload()}>
            Try again
          </button>
        </Notice>
      ) : (
        <p className="text-sm text-muted-foreground">
          {title} offers {num(models.length)} {models.length === 1 ? "model" : "models"}. PULSE picked a strong one for the coding work and a cheaper one for the small jobs; change them if you like.
        </p>
      )}
      <div className="grid gap-5 sm:grid-cols-2">
        <ModelPicker id="wiz-main" label="Main model" note="Does the coding work." value={main} onChange={setMain} models={models} />
        <ModelPicker id="wiz-light" label="Light model" note="Plans, answers and votes. A cheaper model saves money." value={light} onChange={setLight} models={models} sameLabel="Same as the main model" />
      </div>
      <Disclosure summary="Type a model id" open={typing || (!!q.error && !q.data)} onToggle={setTyping} contentClassName="grid gap-3 sm:grid-cols-2">
        <div className="grid gap-1.5">
          <Label htmlFor="wiz-main-id">Main model id</Label>
          <Input id="wiz-main-id" value={main} onChange={(e) => setMain(e.target.value.trim())} placeholder="claude-sonnet-5-5" className="font-mono text-[13px]" spellCheck={false} />
        </div>
        <div className="grid gap-1.5">
          <Label htmlFor="wiz-light-id">Light model id</Label>
          <Input id="wiz-light-id" value={light} onChange={(e) => setLight(e.target.value.trim())} placeholder="Empty: same as the main model" className="font-mono text-[13px]" spellCheck={false} />
        </div>
      </Disclosure>
      <p className="text-[13px] text-muted-foreground">The measuring model, the Ask model and how hard each model thinks are in Settings, under Harness.</p>
    </div>
  );
}

function ReviewRow({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="grid gap-0.5 py-2.5 sm:grid-cols-[9rem_minmax(0,1fr)] sm:gap-4">
      <dt className="text-[13px] text-muted-foreground">{label}</dt>
      <dd className="min-w-0 text-sm anywhere">{children}</dd>
    </div>
  );
}

// The address field starts from the saved setting of a provider: empty
// when it is the provider's own address (the field says "leave it empty"),
// the default for a local server, so the owner sees where PULSE will look.
function startAddress(h: HarnessStatus, name: string): string {
  const info = providerInfo(name);
  const saved = info.baseUrlSetting ? (settingValue(h, info.baseUrlSetting) ?? "") : "";
  const def = info.defaultBaseUrl ?? "";
  return saved && saved !== def ? saved : info.kind === "local" ? def : "";
}

// The wizard's steps live inside the dialog's content, which unmounts when
// the dialog closes: every opening starts fresh, at the provider asked for
// (Connect on the card) or the one in use.
function ProviderSteps({ h, start, onClose, busyRef }: { h: HarnessStatus; start?: string; onClose: () => void; busyRef: React.RefObject<boolean> }) {
  const current = h.provider.name;
  const [step, setStep] = useState<Step>(start ? "connect" : "pick");
  const [reach, setReach] = useState(start ? 1 : 0);
  const [choice, setChoice] = useState(start || current);
  const [address, setAddress] = useState(() => startAddress(h, start || current));
  const [customNoKey, setCustomNoKey] = useState(() => settingValue(h, "provider.custom.no_key") === "on");
  const [login, setLogin] = useState<HarnessLogin | undefined>(undefined);
  const [main, setMain] = useState("");
  const [light, setLight] = useState("");
  const [models, setModels] = useState<HarnessModel[] | null>(null);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState("");
  const body = useRef<HTMLDivElement>(null);

  const info = providerInfo(choice, h.providers.find((p) => p.name === choice)?.label);
  const status = useApi<HarnessProviderInfo>(choice ? harnessProviderPath(choice) : null, { refresh: false });
  // An older hub has no per-provider status: the one in use still reports its own.
  const knownLogin = login ?? status.data?.login ?? (choice === current ? h.provider.login : undefined);
  const local = info.kind === "local" || (choice === "custom" && customNoKey);
  const addressError = address.trim() && !validUrl(address) ? "Enter a full address that starts with http:// or https://." : "";
  const needsAddress = !!info.needsAddress && !address.trim();

  // A different provider starts its connect step from scratch.
  const choose = (name: string) => {
    if (name === choice) return;
    setChoice(name);
    setAddress(startAddress(h, name));
    setLogin(undefined);
    setModels(null);
  };

  // Seed the model picks once the chosen provider's list arrives: keep the
  // current ones when the provider lists them, else suggest.
  const onModels = useCallback(
    (list: HarnessModel[]) => {
      if (models) return;
      setModels(list);
      setMain(suggestModel(list, choice === current ? h.models.worker : ""));
      setLight(suggestLightModel(list, choice === current ? h.models.light : ""));
    },
    [models, choice, current, h.models.worker, h.models.light],
  );

  useEffect(() => {
    body.current?.scrollTo({ top: 0 });
  }, [step]);

  const go = (s: Step) => {
    setStep(s);
    setReach((r) => Math.max(r, STEPS.findIndex((x) => x.id === s)));
  };

  const connectReady = !addressError && !needsAddress && (local || (!!knownLogin?.logged_in && !expiredLogin(knownLogin)));
  const stale = (["measure_model", "ask.model"] as const).filter((k) => {
    const v = settingValue(h, k) ?? (k === "measure_model" ? h.models.measure : h.models.ask) ?? "";
    return !!v && choice !== current && !!models && !models.some((m) => m.id === v);
  });
  const switching = choice !== current;

  const save = async () => {
    busyRef.current = true;
    setSaving(true);
    setSaveError("");
    try {
      const writesList: [string, string][] = [];
      if (info.baseUrlSetting && address.trim() !== (settingValue(h, info.baseUrlSetting) ?? "")) writesList.push([info.baseUrlSetting, address.trim()]);
      if (choice === "custom" && (settingValue(h, "provider.custom.no_key") === "on") !== customNoKey) writesList.push(["provider.custom.no_key", customNoKey ? "on" : "off"]);
      writesList.push(["worker_model", main]);
      writesList.push(["light_model", light]);
      for (const k of stale) writesList.push([k, ""]);
      // The provider goes last: by then its models are in place, so the next run starts on a working pair.
      if (switching) writesList.push(["provider", choice]);
      for (const [k, v] of writesList) await writes.setSetting(k, v);
      invalidate("harness");
      invalidate("settings");
      invalidate(/^harness\/(models|providers)/);
      go("done");
    } catch (e) {
      setSaveError(errorLine(e));
    } finally {
      busyRef.current = false;
      setSaving(false);
    }
  };

  const title = step === "done" ? "All set" : step === "pick" ? "Choose a model provider" : step === "connect" ? "Connect " + info.title : step === "models" ? "Choose the models" : switching ? "Review and switch" : "Review and save";
  const sub =
    step === "pick"
      ? "Where agents get their models. You can change it any time; runs that already started finish as they are."
      : step === "connect"
        ? "PULSE checks the connection before anything changes."
        : step === "models"
          ? "Pick from " + info.title + "'s own list."
          : step === "review"
            ? "Nothing has changed yet."
            : "";

  return (
    <>
        <div className="grid gap-3 border-b px-5 pt-5 pb-4">
          <Stepper steps={STEPS} current={step === "done" ? "review" : step} reach={reach} done={step === "done"} onGo={(id) => go(id as Step)} />
          <div className="pr-8">
            <DialogTitle className="text-lg font-semibold leading-snug">{title}</DialogTitle>
            {sub ? <DialogDescription className="mt-0.5 text-[13px] leading-relaxed text-muted-foreground">{sub}</DialogDescription> : <DialogDescription className="sr-only">The provider is connected.</DialogDescription>}
          </div>
        </div>

        <div ref={body} className="scroll-thin min-h-0 flex-1 overflow-y-auto px-5 py-5">
          {step === "pick" ? <PickStep h={h} choice={choice} onChoose={choose} onNext={() => go("connect")} /> : null}
          {step === "connect" ? <ConnectStep h={h} provider={choice} address={address} setAddress={setAddress} addressError={addressError} login={knownLogin} onLogin={setLogin} customNoKey={customNoKey} setCustomNoKey={setCustomNoKey} reloadLogin={() => void status.reload()} /> : null}
          {step === "models" ? <ModelsStep provider={choice} title={info.title} main={main} setMain={setMain} light={light} setLight={setLight} onModels={onModels} /> : null}
          {step === "review" ? (
            <div className="grid gap-4">
              <dl className="divide-y rounded-xl border bg-card px-4">
                <ReviewRow label="Provider">
                  <span className="flex items-center gap-2">
                    <ProviderGlyph name={choice} title={info.title} className="size-6 text-[10px]" />
                    {info.title}
                    {!switching ? <Badge variant="secondary">In use</Badge> : null}
                  </span>
                </ReviewRow>
                <ReviewRow label="Connected">{local ? "No key needed" : connectedWords(knownLogin)}</ReviewRow>
                {info.baseUrlSetting && (address.trim() || info.kind === "local") ? (
                  <ReviewRow label="Address">
                    <Mono>{address.trim() || info.defaultBaseUrl}</Mono>
                  </ReviewRow>
                ) : null}
                <ReviewRow label="Main model">{main ? <Mono>{main}</Mono> : "The provider's default"}</ReviewRow>
                <ReviewRow label="Light model">{light ? <Mono>{light}</Mono> : "Same as the main model"}</ReviewRow>
              </dl>
              {switching ? (
                <Notice tone="info">
                  New runs start on {info.title}. Runs that already started finish on {providerInfo(current, h.provider.label).title}.
                  {stale.length ? " The " + stale.map((k) => (k === "measure_model" ? "measuring" : "Ask")).join(" and ") + " model came from the old provider, so it goes back to following the main model." + (stale.includes("measure_model") ? " Self-improvement measures its baselines again with the new model." : "") : ""}
                </Notice>
              ) : null}
              {saveError ? <Notice tone="bad">{saveError}</Notice> : null}
            </div>
          ) : null}
          {step === "done" ? (
            <div className="grid justify-items-center gap-3 py-6 text-center">
              <span className="inline-flex size-12 items-center justify-center rounded-full bg-ok/12 text-ok">
                <CircleCheckIcon aria-hidden className="size-7" />
              </span>
              <p className="text-lg font-semibold">Agents now use {info.title}</p>
              <p className="max-w-md text-sm text-muted-foreground">
                {main ? (
                  <>
                    Coding work runs on <Mono>{main}</Mono>
                    {light ? (
                      <>
                        , the small jobs on <Mono>{light}</Mono>
                      </>
                    ) : null}
                    .
                  </>
                ) : (
                  "Runs use " + info.title + "'s default model."
                )}{" "}
                The next run picks it up.
              </p>
              <Button asChild variant="outline" size="sm">
                <Link href={hrefs.harness()} onClick={() => onClose()}>
                  Watch the runs <ArrowRightIcon />
                </Link>
              </Button>
            </div>
          ) : null}
        </div>

        <div className="flex items-center justify-between gap-2 border-t bg-muted/30 px-5 py-3">
          {step === "pick" || step === "done" ? (
            <span className="text-[12.5px] text-muted-foreground">{step === "pick" ? <CloudIcon aria-hidden className="mr-1.5 inline size-3.5 -translate-y-px" /> : null}{step === "pick" ? "Keys and sign-ins stay on this machine." : ""}</span>
          ) : (
            <Button variant="ghost" onClick={() => go(step === "connect" ? "pick" : step === "models" ? "connect" : "models")} disabled={saving}>
              <ArrowLeftIcon /> Back
            </Button>
          )}
          {step === "pick" ? (
            <Button onClick={() => go("connect")} disabled={!choice}>
              Continue <ArrowRightIcon />
            </Button>
          ) : step === "connect" ? (
            <Button
              onClick={() => {
                // A local server's address is saved by Check the connection; save it here too so the models step lists from it.
                const key = info.baseUrlSetting;
                const p = key && address.trim() !== (settingValue(h, key) ?? "") ? writes.setSetting(key, address.trim()).then(() => invalidate("harness")) : Promise.resolve();
                void p.then(() => {
                  invalidate(harnessModelsPath(choice));
                  go("models");
                });
              }}
              disabled={!connectReady}
              title={connectReady ? undefined : needsAddress ? "Enter the address first" : "Connect first"}
            >
              Continue <ArrowRightIcon />
            </Button>
          ) : step === "models" ? (
            <Button onClick={() => go("review")}>
              Continue <ArrowRightIcon />
            </Button>
          ) : step === "review" ? (
            <Button onClick={() => void save()} disabled={saving}>
              {saving ? <RefreshCwIcon className="animate-spin" /> : <CheckIcon />} {switching ? "Switch to " + info.title : "Save"}
            </Button>
          ) : (
            <Button onClick={() => onClose()}>Done</Button>
          )}
        </div>
    </>
  );
}

export function ProviderWizard({ open, onOpenChange, h, start }: { open: boolean; onOpenChange: (open: boolean) => void; h: HarnessStatus; start?: string }) {
  // While the switch is being saved the dialog stays open.
  const busy = useWizardLock();
  return (
    <WizardDialog open={open} onOpenChange={(o) => !busy.current && onOpenChange(o)}>
      <ProviderSteps h={h} start={start} onClose={() => onOpenChange(false)} busyRef={busy} />
    </WizardDialog>
  );
}
