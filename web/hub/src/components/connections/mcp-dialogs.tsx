"use client";

import Link from "next/link";
import { useEffect, useMemo, useRef, useState } from "react";
import { ArrowLeftIcon, ArrowRightIcon, CheckCircle2Icon, CheckIcon, CircleAlertIcon, CopyIcon, ExternalLinkIcon, KeyRoundIcon, Loader2Icon, PlugZapIcon, PlusIcon, SearchIcon, SparklesIcon, TrashIcon, WrenchIcon } from "lucide-react";
import { toast } from "sonner";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import { showApiError } from "@/components/pulse/confirm";
import { copyText } from "@/components/pulse/evolve";
import { PulseMascot } from "@/components/pulse/mascots";
import { Disclosure, Notice } from "@/components/pulse/panel";
import { LogoTile } from "@/components/pulse/logo";
import { ChoiceCard, WizardBody, WizardDialog, WizardFooter, WizardHeader, useWizardLock, type WizardStep } from "@/components/pulse/stepper";
import { Mono } from "@/components/pulse/who";
import { api, enc, errorText, writes, type McpAuth, type McpConnectResult, type McpLoginFlow, type McpLoginInput, type McpLoginState, type McpOptions, type McpServer, type McpServersPayload, type McpTransport, type ProjectsPayload, type ToolkitPayload, type ToolSuggestion } from "@/lib/api";
import { EMPTY_MCP_FORM, TRANSPORT_BLURB, TRANSPORT_LABEL, authExplain, authLabel, authFieldLabel, authFieldPlaceholder, authsFor, formFromServer, mcpInput, mcpPatch, mcpFormReady, needsSecret, secretHint, validateMcpForm, type EnvRow, type McpForm } from "@/lib/mcp";
import { formFromPreset, presetAccess, toolChoices, type McpPreset } from "@/lib/mcp-presets";
import { hrefs } from "@/lib/nav";
import { invalidate, useApi } from "@/lib/use-api";
import { cn } from "@/lib/utils";

// The owner registers an MCP server the way `pulse mcp add` does: a name,
// the project it serves (or every project), how to reach it, how it
// authenticates, and where the secret lives - a pulse-cred reference like
// the push key's, never the secret. The hub validates again and records
// mcp.added / mcp.updated. A new tool goes through a short wizard: pick a
// well-known one (or start blank), check the details it filled in, then
// PULSE saves it and connects to it, so the owner sees what it offers
// before closing. Editing a saved one is the plain form.

export const refreshMcp = () => invalidate((p) => p === "mcp" || p.startsWith("mcp?") || p.startsWith("mcp/") || p.startsWith("events"));

const ALL = "__all__";

// `initial` pre-fills a new server (the toolkit's suggestions use it): the
// wizard then opens at its details. `onSignIn` is how the page offers the
// one-time sign-in of an OAuth tool right after it is saved.
export function McpServerDialog({ open, onOpenChange, options, server, project, initial, onSignIn }: { open: boolean; onOpenChange: (o: boolean) => void; options: McpOptions; server?: McpServer | null; project?: string; initial?: Partial<McpForm>; onSignIn?: (s: McpServer) => void }) {
  const lock = useWizardLock();
  if (server)
    return (
      <Dialog open={open} onOpenChange={onOpenChange}>
        <DialogContent className="sm:max-w-2xl">{open ? <McpFormBody options={options} server={server} close={() => onOpenChange(false)} /> : null}</DialogContent>
      </Dialog>
    );
  return (
    <WizardDialog open={open} onOpenChange={(o) => !lock.current && onOpenChange(o)}>
      <ToolWizard options={options} project={project ?? ""} initial={initial} close={() => onOpenChange(false)} lockRef={lock} onSignIn={onSignIn} />
    </WizardDialog>
  );
}

// Editing a saved tool: every field at once, as before.
function McpFormBody({ options, server, close }: { options: McpOptions; server: McpServer; close: () => void }) {
  const [f, setF] = useState<McpForm>(() => formFromServer(server));
  const [busy, setBusy] = useState(false);
  const [touched, setTouched] = useState(false);
  const errors = validateMcpForm(f, options);

  const submit = async () => {
    setTouched(true);
    if (errors.length) return;
    setBusy(true);
    const id = toast.loading("Saving " + server.name + "…");
    try {
      const patch = mcpPatch(server, f);
      if (!Object.keys(patch).length) {
        toast.dismiss(id);
        toast.message("Nothing changed.");
        close();
        return;
      }
      const r = await writes.updateMcp(server.id, patch);
      toast.success(r.server.name + " saved.", { id, description: "Check it to make sure it still answers." });
      refreshMcp();
      close();
    } catch (e) {
      toast.dismiss(id);
      showApiError(e, "The tool wasn't saved");
    } finally {
      setBusy(false);
    }
  };

  return (
    <>
      <DialogHeader>
        <DialogTitle>{"Edit " + server.name}</DialogTitle>
        <DialogDescription>How PULSE reaches the tool and signs in to it. If it needs a secret, the secret stays where you keep it; PULSE only stores where to find it.</DialogDescription>
      </DialogHeader>
      <McpFields f={f} setF={setF} options={options} />
      {touched && errors.length ? <Problems errors={errors} /> : null}
      <DialogFooter className="items-center gap-2">
        <Button variant="ghost" onClick={close} disabled={busy}>
          Cancel
        </Button>
        <Button onClick={() => void submit()} disabled={busy || !mcpFormReady(f) || (touched && errors.length > 0)}>
          {busy ? <Loader2Icon className="animate-spin" /> : <CheckIcon />}
          Save
        </Button>
      </DialogFooter>
    </>
  );
}

function Problems({ errors }: { errors: string[] }) {
  return (
    <Notice tone="warn">
      <ul className="list-disc pl-4">
        {errors.map((e) => (
          <li key={e}>{e}</li>
        ))}
      </ul>
    </Notice>
  );
}

// The fields of a tool. `simple` (a well-known tool, already filled in)
// keeps the parts the owner rarely changes - how it is started or reached,
// the sign-in method - folded under "How PULSE reaches it"; what still
// needs the owner (the secret's place, who may use it) stays open.
function McpFields({ f, setF, options, simple }: { f: McpForm; setF: React.Dispatch<React.SetStateAction<McpForm>>; options: McpOptions; simple?: boolean }) {
  const { data: projects } = useApi<ProjectsPayload>("projects", { refresh: false });
  const set = <K extends keyof McpForm>(k: K, v: McpForm[K]) => setF((cur) => ({ ...cur, [k]: v }));
  const auths = authsFor(options, f.transport);
  const fieldLabel = authFieldLabel(f.auth);
  const slugs = (projects?.projects ?? []).filter((p) => p.status === "active").map((p) => p.slug);
  const knownProject = !f.project || slugs.includes(f.project);
  // Changing the transport can leave an auth that does not fit; fall back to none.
  const setTransport = (t: McpTransport) => {
    setF((cur) => ({ ...cur, transport: t, auth: authsFor(options, t).some((a) => a.name === cur.auth) ? cur.auth : "none" }));
  };

  const reach = (
    <>
      <fieldset className="grid gap-2">
        <legend className="mb-1.5 text-sm font-medium">How PULSE reaches it</legend>
        <div className="grid gap-2 sm:grid-cols-3">
          {options.transports.map((t) => (
            <label key={t} className={cn("flex cursor-pointer items-start gap-2.5 rounded-lg border bg-card px-3 py-2.5 text-left transition-colors has-[:checked]:border-primary/60 has-[:checked]:bg-primary/5", "hover:bg-muted/50")}>
              <input type="radio" name="mcp-transport" value={t} checked={f.transport === t} onChange={() => setTransport(t)} className="mt-1 size-3.5 shrink-0 accent-primary" />
              <span className="min-w-0">
                <span className="block text-sm font-medium">{TRANSPORT_LABEL[t]}</span>
                <span className="block text-xs leading-snug text-muted-foreground">{TRANSPORT_BLURB[t]}</span>
              </span>
            </label>
          ))}
        </div>
      </fieldset>

      {f.transport === "stdio" ? (
        <div className="grid gap-3 sm:grid-cols-[minmax(0,1fr)_minmax(0,1fr)]">
          <div className="grid content-start gap-1.5">
            <Label htmlFor="mcp-command">Program</Label>
            <Input id="mcp-command" value={f.command} onChange={(e) => set("command", e.target.value)} placeholder="e.g. npx" spellCheck={false} className="font-mono text-[13px]" />
            <p className="text-xs text-muted-foreground">The program PULSE starts on this machine, by name or full path. A whole line such as <code>npx -y @azure/mcp@latest server start</code> works too.</p>
          </div>
          <div className="grid content-start gap-1.5">
            <Label htmlFor="mcp-args">What follows it</Label>
            <Textarea id="mcp-args" value={f.args} onChange={(e) => set("args", e.target.value)} rows={3} placeholder={"e.g. -y\n@modelcontextprotocol/server-github"} spellCheck={false} className="min-h-[4.5rem] font-mono text-[13px]" />
            <p className="text-xs text-muted-foreground">One per line, or separated by spaces as in a shell.</p>
          </div>
        </div>
      ) : (
        <div className="grid content-start gap-1.5">
          <Label htmlFor="mcp-url">Web address</Label>
          <Input id="mcp-url" value={f.url} onChange={(e) => set("url", e.target.value)} placeholder="e.g. https://mcp.example.com/mcp" spellCheck={false} className="font-mono text-[13px]" />
          <p className="text-xs text-muted-foreground">{f.transport === "http" ? "Where the server answers, often ending in /mcp." : "The address of the event stream."}</p>
        </div>
      )}

      <div className="grid gap-3 sm:grid-cols-2">
        <div className="grid content-start gap-1.5">
          <Label htmlFor="mcp-auth">Sign-in</Label>
          <Select value={f.auth} onValueChange={(v) => set("auth", v as McpAuth)}>
            <SelectTrigger id="mcp-auth" className="w-full">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {auths.map((a) => (
                <SelectItem key={a.name} value={a.name}>
                  {authLabel(a.name)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <p className="text-xs text-muted-foreground anywhere">{authExplain(options, f.auth)}</p>
        </div>
        {fieldLabel ? (
          <div className="grid content-start gap-1.5">
            <Label htmlFor="mcp-auth-header">{fieldLabel}</Label>
            <Input id="mcp-auth-header" value={f.auth_header} onChange={(e) => set("auth_header", e.target.value)} placeholder={"e.g. " + authFieldPlaceholder(f.auth)} spellCheck={false} className="font-mono text-[13px]" />
            <p className="text-xs text-muted-foreground">
              {f.auth === "env" ? "PULSE sets this variable for the program, with the secret as its value." : f.auth === "query" ? "PULSE adds this parameter to the address, with the secret as its value." : "PULSE sends this header with every request, with the secret as its value."}
            </p>
          </div>
        ) : null}
      </div>
      {f.transport === "stdio" ? <EnvEditor rows={f.env} onChange={(rows) => set("env", rows)} /> : null}
    </>
  );

  return (
    <div className="grid gap-4">
      <div className="grid gap-3 sm:grid-cols-2">
        <div className="grid content-start gap-1.5">
          <Label htmlFor="mcp-name">Name</Label>
          <Input id="mcp-name" value={f.name} onChange={(e) => set("name", e.target.value)} placeholder="e.g. github" maxLength={64} spellCheck={false} className="font-mono text-[13px]" />
          <p className="text-xs text-muted-foreground">Short and lowercase. Agents call the tool by this name.</p>
        </div>
        <div className="grid content-start gap-1.5">
          <Label htmlFor="mcp-project">Who can use it</Label>
          <Select value={f.project || ALL} onValueChange={(v) => set("project", v === ALL ? "" : v)}>
            <SelectTrigger id="mcp-project" className="w-full">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value={ALL}>All projects</SelectItem>
              {!knownProject ? <SelectItem value={f.project}>{f.project}</SelectItem> : null}
              {slugs.map((slug) => (
                <SelectItem key={slug} value={slug}>
                  {slug}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <p className="text-xs text-muted-foreground">Agents of every project, or of one. A project&apos;s own tool replaces a shared one with the same name.</p>
        </div>
      </div>

      {simple ? (
        <Disclosure summary="How PULSE reaches it" hint={f.transport === "stdio" ? "Starts " + (f.command || "a program") + " here" : f.url ? hostOf(f.url) : ""} contentClassName="grid gap-4">
          {reach}
        </Disclosure>
      ) : (
        reach
      )}

      {f.auth === "oauth" ? (
        <Notice tone="info">
          You sign in once after saving: PULSE opens the sign-in page and keeps the tokens on this machine, renewing them itself. There&apos;s no secret to store.
        </Notice>
      ) : null}

      {needsSecret(f.auth) ? (
        <div className="grid content-start gap-1.5">
          <Label htmlFor="mcp-secret">Where the secret is kept</Label>
          <Input id="mcp-secret" value={f.secret_ref} onChange={(e) => set("secret_ref", e.target.value)} placeholder="e.g. op://PULSE/github-mcp/token" spellCheck={false} className="font-mono text-[13px]" />
          <p className="text-xs text-muted-foreground anywhere">
            Where to find it, never the secret itself: <span className="font-mono text-xs">{options.cred_shapes}</span>. {secretHint(f.auth)} The 1Password token saved in{" "}
            <Link href={hrefs.settings("runner")} className="underline underline-offset-2">
              Settings
            </Link>{" "}
            also unlocks op:// references here.
          </p>
        </div>
      ) : null}

      <div className="grid content-start gap-1.5">
        <Label htmlFor="mcp-note">What it&apos;s for</Label>
        <Textarea id="mcp-note" value={f.note} onChange={(e) => set("note", e.target.value)} rows={2} placeholder="e.g. Issues and pull requests of every project; read before planning." className="min-h-[3.5rem]" />
        <p className="text-xs text-muted-foreground">Agents read this next to the name. Say what the tool is for and when to use it.</p>
      </div>
    </div>
  );
}

type ToolStep = "pick" | "details" | "check" | "done";
const TOOL_STEPS: WizardStep[] = [
  { id: "pick", label: "Choose" },
  { id: "details", label: "Details" },
  { id: "check", label: "Connect" },
];

function ToolWizard({ options, project, initial, close, lockRef, onSignIn }: { options: McpOptions; project: string; initial?: Partial<McpForm>; close: () => void; lockRef: React.RefObject<boolean>; onSignIn?: (s: McpServer) => void }) {
  const [step, setStep] = useState<ToolStep>(initial ? "details" : "pick");
  const [reach, setReach] = useState(initial ? 1 : 0);
  const [choice, setChoice] = useState<string>(initial?.name ? "preset:" + initial.name : "");
  const [simple, setSimple] = useState(!!initial);
  const [f, setF] = useState<McpForm>(() => ({ ...EMPTY_MCP_FORM, project, ...initial }));
  const [saved, setSaved] = useState<McpServer | null>(null);
  const [busy, setBusy] = useState(false);
  const [touched, setTouched] = useState(false);
  const [q, setQ] = useState("");
  const body = useRef<HTMLDivElement>(null);
  const toolkit = useApi<ToolkitPayload>("toolkit", { refresh: false });
  const servers = useApi<McpServersPayload>("mcp", { refresh: false });
  const errors = validateMcpForm(f, options);

  const suggestions: ToolSuggestion[] = useMemo(() => {
    const d = toolkit.data;
    if (!d) return [];
    return (d.projects ?? [{ project: d.project, tools: d.tools ?? [], skills: [] }]).flatMap((g) => g.tools);
  }, [toolkit.data]);
  const taken = (servers.data?.servers ?? []).map((s) => s.name);
  const { suggested, popular } = toolChoices(suggestions, taken);
  const needle = q.trim().toLowerCase();
  const hit = (p: McpPreset) => !needle || (p.name + " " + p.title + " " + p.about).toLowerCase().includes(needle);
  const all = [...suggested, ...popular];

  useEffect(() => {
    body.current?.scrollTo({ top: 0 });
  }, [step]);
  const go = (s: ToolStep) => {
    setStep(s);
    setReach((r) => Math.max(r, TOOL_STEPS.findIndex((x) => x.id === s)));
  };

  const pick = (key: string) => {
    setChoice(key);
    setTouched(false);
    if (key === "custom") {
      setSimple(false);
      setF({ ...EMPTY_MCP_FORM, project });
      return;
    }
    const p = all.find((x) => "preset:" + x.name === key);
    if (p) {
      setSimple(true);
      setF({ ...EMPTY_MCP_FORM, ...formFromPreset(p, project) });
    }
  };

  // Save (a new tool, or the one this wizard saved already) and go connect.
  const save = async () => {
    setTouched(true);
    if (errors.length) return;
    setBusy(true);
    lockRef.current = true;
    try {
      if (saved) {
        const patch = mcpPatch(saved, f);
        const r = Object.keys(patch).length ? await writes.updateMcp(saved.id, patch) : { server: saved };
        setSaved(r.server);
      } else {
        const r = await writes.addMcp(mcpInput(f));
        setSaved(r.server);
      }
      refreshMcp();
      go("check");
    } catch (e) {
      showApiError(e, saved ? "The tool wasn't saved" : "The tool wasn't added");
    } finally {
      lockRef.current = false;
      setBusy(false);
    }
  };

  const chosen = all.find((x) => "preset:" + x.name === choice);
  const title = step === "pick" ? "Add a tool" : step === "details" ? (chosen ? "Add " + chosen.title : f.name ? "Add " + f.name : "Describe the tool") : step === "check" ? (saved?.auth === "oauth" ? "Sign in to " + saved.name : "Connect " + (saved?.name ?? "the tool")) : "Done";
  const description =
    step === "pick"
      ? "A tool lets agents reach another system: issues, docs, a browser, your database. Pick one and PULSE fills in the details."
      : step === "details"
        ? simple
          ? "PULSE filled this in. Check who may use it" + (needsSecret(f.auth) ? " and say where the secret is kept." : ".")
          : "How PULSE starts or reaches the server, and how it signs in."
        : step === "check"
          ? "Saved. PULSE asks the tool what it offers; agents see the answer in their brief."
          : "";
  const why = step === "pick" ? (!choice ? "Pick a tool, or start blank." : "") : step === "details" ? (!mcpFormReady(f) ? (f.transport === "stdio" ? "Name it and say which program starts it." : "Name it and give its web address.") : touched && errors.length ? "Fix what's listed above." : "") : "";

  return (
    <>
      <WizardHeader steps={TOOL_STEPS} current={step === "done" ? "check" : step} reach={reach} done={step === "done"} onGo={(id) => go(id as ToolStep)} title={title} description={description} />
      <WizardBody bodyRef={body}>
        {step === "pick" ? (
          <div className="grid gap-5">
            <div className="relative">
              <SearchIcon aria-hidden className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground" />
              <Input value={q} onChange={(e) => setQ(e.target.value)} placeholder="Search tools" aria-label="Search tools" className="h-10 pl-9" type="search" />
            </div>
            {suggested.filter(hit).length ? (
              <PresetGroup title="Suggested for your projects" note="From what the projects' files show." items={suggested.filter(hit)} choice={choice} onPick={pick} onNext={() => go("details")} suggested />
            ) : null}
            {popular.filter(hit).length ? <PresetGroup title="Popular tools" note="Well-known servers, ready to use." items={popular.filter(hit)} choice={choice} onPick={pick} onNext={() => go("details")} /> : null}
            {needle && !all.some(hit) ? <p className="text-sm text-muted-foreground">No well-known tool matches &ldquo;{q.trim()}&rdquo;. Start blank instead.</p> : null}
            <section className="grid gap-2" aria-label="Something else">
              <h3 className="text-sm font-semibold">Something else</h3>
              <div role="radiogroup" aria-label="Something else" className="grid gap-2 sm:grid-cols-2">
                <ChoiceCard selected={choice === "custom"} onSelect={() => pick("custom")} icon={WrenchIcon} title="Start blank">
                  Any other MCP server: a program to run here, or a web address.
                </ChoiceCard>
              </div>
            </section>
          </div>
        ) : null}

        {step === "details" ? (
          <div className="grid gap-4">
            {chosen?.why ? (
              <Notice tone="info">
                <span className="font-medium">Why it&apos;s suggested: </span>
                {chosen.why}
              </Notice>
            ) : null}
            {chosen?.secret ? <p className="text-[13px] text-muted-foreground">It needs {chosen.secret}. Keep it in your password manager or keychain and point to it below.</p> : null}
            <McpFields f={f} setF={setF} options={options} simple={simple} />
            {chosen?.docs ? (
              <a href={chosen.docs} target="_blank" rel="noreferrer" className="inline-flex w-fit items-center gap-1 text-[13px] font-medium text-primary hover:underline">
                About {chosen.title} <ExternalLinkIcon className="size-3.5" aria-hidden />
              </a>
            ) : null}
            {touched && errors.length ? <Problems errors={errors} /> : null}
          </div>
        ) : null}

        {step === "check" && saved ? (
          saved.auth === "oauth" ? (
            <div className="grid gap-3">
              <Notice tone="ok">{saved.name} is saved.</Notice>
              <p className="text-sm leading-relaxed text-muted-foreground">One step left: sign in once, so PULSE can call it as you. PULSE keeps the tokens on this machine and renews them.</p>
              {onSignIn ? null : (
                <p className="text-sm leading-relaxed text-muted-foreground">
                  Use <span className="font-medium text-foreground">Sign in</span> on its card, or run <Mono>{"pulse mcp login " + saved.name}</Mono> on this machine.
                </p>
              )}
            </div>
          ) : (
            <ConnectCheck server={saved} onFix={() => go("details")} />
          )
        ) : null}
      </WizardBody>
      <WizardFooter
        left={
          step === "pick" ? (
            <Button variant="ghost" onClick={close}>
              Not now
            </Button>
          ) : step === "details" ? (
            <Button variant="ghost" onClick={() => go("pick")} disabled={busy || !!saved}>
              <ArrowLeftIcon /> Back
            </Button>
          ) : step === "check" ? (
            <Button variant="ghost" onClick={() => go("details")}>
              <ArrowLeftIcon /> Change the details
            </Button>
          ) : null
        }
        why={why}
        right={
          step === "pick" ? (
            <Button onClick={() => go("details")} disabled={!choice}>
              Continue <ArrowRightIcon />
            </Button>
          ) : step === "details" ? (
            <Button onClick={() => void save()} disabled={busy || !mcpFormReady(f) || (touched && errors.length > 0)}>
              {busy ? <Loader2Icon className="animate-spin" /> : <PlugZapIcon />} {saved ? "Save and connect" : f.auth === "oauth" ? "Save" : "Add and connect"}
            </Button>
          ) : saved?.auth === "oauth" && onSignIn ? (
            <Button
              onClick={() => {
                close();
                onSignIn(saved);
              }}
            >
              <KeyRoundIcon /> Sign in now
            </Button>
          ) : (
            <Button onClick={close}>Done</Button>
          )
        }
      />
    </>
  );
}

function PresetGroup({ title, note, items, choice, onPick, onNext, suggested }: { title: string; note: string; items: McpPreset[]; choice: string; onPick: (key: string) => void; onNext: () => void; suggested?: boolean }) {
  return (
    <section className="grid gap-2" aria-label={title}>
      <div className="flex items-baseline gap-2">
        {suggested ? <SparklesIcon aria-hidden className="size-4 translate-y-0.5 text-primary" /> : null}
        <h3 className="text-sm font-semibold">{title}</h3>
        <p className="hidden min-w-0 truncate text-[13px] text-muted-foreground sm:block">{note}</p>
      </div>
      <div role="radiogroup" aria-label={title} className="grid gap-2 sm:grid-cols-2">
        {items.map((p) => {
          const key = "preset:" + p.name;
          const on = choice === key;
          return (
            <button
              key={key}
              type="button"
              role="radio"
              aria-checked={on}
              onClick={() => onPick(key)}
              onDoubleClick={() => {
                onPick(key);
                onNext();
              }}
              className={cn("grid min-w-0 gap-1 rounded-lg border bg-card p-3 text-left transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring", on ? "border-primary ring-1 ring-primary" : "hover:border-foreground/25 hover:bg-muted/40")}
            >
              <span className="flex min-w-0 items-center gap-2">
                <LogoTile name={p.name} size={28} title={p.title} />
                <span className="min-w-0 flex-1 truncate text-sm font-semibold">{p.title}</span>
                <Badge variant="outline" className="shrink-0">
                  {presetAccess(p)}
                </Badge>
              </span>
              <span className="line-clamp-2 text-[13px] leading-snug text-muted-foreground">{suggested && p.project ? "For " + p.project + ". " : ""}{p.about}</span>
            </button>
          );
        })}
      </div>
    </section>
  );
}


// The wizard's last step for a tool without OAuth: connect, list what it
// offers, or say plainly what went wrong.
function ConnectCheck({ server, onFix }: { server: McpServer; onFix: () => void }) {
  const [run, setRun] = useState<{ attempt: number; state: ConnectState }>({ attempt: 0, state: { kind: "running" } });
  const { attempt, state } = run;
  useEffect(() => {
    let cancelled = false;
    writes
      .connectMcp(server.id)
      .then((result) => {
        if (cancelled) return;
        setRun({ attempt, state: { kind: "ok", result } });
        refreshMcp();
      })
      .catch((e: unknown) => {
        if (cancelled) return;
        setRun({ attempt, state: { kind: "failed", ...errorText(e) } });
        refreshMcp();
      });
    return () => {
      cancelled = true;
    };
  }, [server.id, attempt]);
  if (state.kind === "running")
    return (
      <div className="flex items-center gap-2 py-6 text-sm text-muted-foreground" role="status" aria-live="polite">
        <Loader2Icon className="size-4 motion-safe:animate-spin" aria-hidden /> Connecting to {server.name}…
      </div>
    );
  if (state.kind === "ok")
    return (
      <div className="grid gap-3">
        <div className="flex items-center gap-3">
          <span className="inline-flex size-10 shrink-0 items-center justify-center rounded-full bg-ok/12 text-ok">
            <CheckCircle2Icon aria-hidden className="size-6" />
          </span>
          <div className="min-w-0">
            <p className="font-semibold">{"Connected to " + (state.result.info.name || server.name)}</p>
            <p className="text-[13px] text-muted-foreground">{state.result.tools.length ? (state.result.tools.length === 1 ? "It offers 1 tool. " : "It offers " + state.result.tools.length + " tools. ") + "Agents see it from their next job." : "It answered, but offers no tools yet."}</p>
          </div>
        </div>
        {state.result.tools.length ? (
          <ul className="grid max-h-64 gap-1.5 overflow-y-auto pr-1">
            {state.result.tools.map((t) => (
              <li key={t.name} className="rounded-lg border bg-card px-3 py-2">
                <div className="flex items-center gap-2">
                  <WrenchIcon className="size-3.5 shrink-0 text-muted-foreground" aria-hidden />
                  <Mono>{t.name}</Mono>
                </div>
                {t.description ? <p className="mt-1 line-clamp-2 text-xs leading-snug text-muted-foreground anywhere">{t.description}</p> : null}
              </li>
            ))}
          </ul>
        ) : null}
      </div>
    );
  return (
    <div className="grid gap-3">
      <div className="flex items-start gap-3">
        <span className="inline-flex size-10 shrink-0 items-center justify-center rounded-full bg-bad/10 text-bad">
          <CircleAlertIcon aria-hidden className="size-6" />
        </span>
        <div className="min-w-0">
          <p className="font-semibold">{server.name} is saved, but didn&apos;t answer</p>
          <p className="text-[13px] leading-relaxed text-muted-foreground anywhere">
            {state.error}
            {state.why ? ". " + state.why : ""}
            {state.fix ? " " + state.fix : ""}
          </p>
        </div>
      </div>
      <div className="flex flex-wrap gap-2">
        <Button size="sm" variant="outline" onClick={() => setRun((cur) => ({ attempt: cur.attempt + 1, state: { kind: "running" } }))}>
          Try again
        </Button>
        <Button size="sm" variant="ghost" onClick={onFix}>
          Change the details
        </Button>
      </div>
      <p className="text-[13px] text-muted-foreground">Or keep it and fix it later: its card says what went wrong and has Check now.</p>
    </div>
  );
}

// Environment for a stdio command. A value may be a pulse-cred reference,
// resolved when the server starts, so a token never sits in the store.
function EnvEditor({ rows, onChange }: { rows: EnvRow[]; onChange: (rows: EnvRow[]) => void }) {
  const update = (i: number, patch: Partial<EnvRow>) => onChange(rows.map((r, j) => (j === i ? { ...r, ...patch } : r)));
  return (
    <div className="grid content-start gap-1.5">
      <div className="flex items-center justify-between">
        <Label>Environment variables</Label>
        <Button type="button" size="xs" variant="outline" onClick={() => onChange([...rows, { key: "", value: "" }])}>
          <PlusIcon /> Add a variable
        </Button>
      </div>
      {rows.length ? (
        <div className="grid content-start gap-1.5">
          {rows.map((r, i) => (
            <div key={i} className="grid grid-cols-[minmax(0,2fr)_minmax(0,3fr)_auto] items-center gap-1.5">
              <Input aria-label={"Variable " + (i + 1) + " name"} value={r.key} onChange={(e) => update(i, { key: e.target.value })} placeholder="GITHUB_HOST" spellCheck={false} className="font-mono text-xs" />
              <Input aria-label={"Variable " + (i + 1) + " value"} value={r.value} onChange={(e) => update(i, { value: e.target.value })} placeholder="value or op://… reference" spellCheck={false} className="font-mono text-xs" />
              <Button type="button" size="icon-sm" variant="ghost" onClick={() => onChange(rows.filter((_, j) => j !== i))} aria-label={"Remove variable " + (r.key || i + 1)}>
                <TrashIcon />
              </Button>
            </div>
          ))}
        </div>
      ) : null}
      <p className="text-xs text-muted-foreground">Plain values, or references (op://, keychain://, file://) PULSE reads when the program starts. A value that looks like a token is refused: use a reference instead.</p>
    </div>
  );
}

// ----- connect ---------------------------------------------------------------

type ConnectState = { kind: "running" } | { kind: "ok"; result: McpConnectResult } | { kind: "failed"; error: string; why: string; fix: string };

// Connect = initialize + tools/list, recorded on the row as its last check.
// The dialog opens running, then shows the server's name and version with
// its tools, or what went wrong and what to do about it.
export function ConnectDialog({ server, onOpenChange, onSignIn }: { server: McpServer | null; onOpenChange: (o: boolean) => void; onSignIn?: (s: McpServer) => void }) {
  return (
    <Dialog open={!!server} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-xl">{server ? <ConnectBody server={server} onSignIn={onSignIn} /> : null}</DialogContent>
    </Dialog>
  );
}

// A failed check of an `auth: oauth` server that points at `pulse mcp login`
// is a missing or expired sign-in; the dialog offers to run it here.
const wantsSignIn = (server: McpServer, fix: string): boolean => server.auth === "oauth" && /pulse mcp login/.test(fix);

function ConnectBody({ server, onSignIn }: { server: McpServer; onSignIn?: (s: McpServer) => void }) {
  // One attempt per (server, counter); "Try again" bumps the counter and
  // resets the view in the same click, so the effect only reports results.
  const [run, setRun] = useState<{ attempt: number; state: ConnectState }>({ attempt: 0, state: { kind: "running" } });
  const state = run.state;
  const attempt = run.attempt;
  useEffect(() => {
    let cancelled = false;
    writes
      .connectMcp(server.id)
      .then((result) => {
        if (cancelled) return;
        setRun({ attempt, state: { kind: "ok", result } });
        refreshMcp();
      })
      .catch((e: unknown) => {
        if (cancelled) return;
        setRun({ attempt, state: { kind: "failed", ...errorText(e) } });
        refreshMcp();
      });
    return () => {
      cancelled = true;
    };
  }, [server.id, attempt]);
  return (
    <>
      <DialogHeader>
        <DialogTitle className="flex items-center gap-2">
          <PlugZapIcon className="size-4 text-primary" aria-hidden /> Checking {server.name}
        </DialogTitle>
        <DialogDescription>PULSE connects to the tool and asks what it offers. The answer is saved with the tool and shown to agents in their brief.</DialogDescription>
      </DialogHeader>
      {state.kind === "running" ? (
        <div className="flex items-center gap-2 py-6 text-sm text-muted-foreground" role="status" aria-live="polite">
          <Loader2Icon className="size-4 motion-safe:animate-spin" aria-hidden /> Connecting to {server.name}…
        </div>
      ) : state.kind === "ok" ? (
        <div className="grid gap-3">
          <Notice tone="ok">
            <span className="font-medium">{"Connected to " + (state.result.info.name || server.name)}</span>
            {state.result.info.version ? <span title={state.result.info.protocol_version ? "MCP protocol " + state.result.info.protocol_version : undefined}>{" (version " + state.result.info.version + ")"}</span> : null}
            <span className="block text-xs text-muted-foreground anywhere">{state.result.note}</span>
          </Notice>
          {state.result.info.instructions ? <p className="text-sm leading-relaxed text-muted-foreground anywhere">{state.result.info.instructions}</p> : null}
          {state.result.tools.length ? (
            <>
              <p className="text-sm font-medium">{state.result.tools.length === 1 ? "1 tool it offers" : state.result.tools.length + " tools it offers"}</p>
              <ul className="grid max-h-80 gap-1.5 overflow-y-auto pr-1">
                {state.result.tools.map((t) => (
                  <li key={t.name} className="rounded-lg border bg-card px-3 py-2">
                    <div className="flex items-center gap-2">
                      <WrenchIcon className="size-3.5 shrink-0 text-muted-foreground" aria-hidden />
                      <Mono>{t.name}</Mono>
                    </div>
                    {t.description ? <p className="mt-1 text-xs leading-snug text-muted-foreground anywhere">{t.description}</p> : null}
                  </li>
                ))}
              </ul>
            </>
          ) : (
            <p className="text-sm text-muted-foreground">The server answered, but it offers no tools yet.</p>
          )}
          <Disclosure summary="How agents use it" contentClassName="mt-1.5 text-xs leading-relaxed text-muted-foreground">
            <p>
              Agents call a tool with{" "}
              <Mono>
                pulse mcp call {server.name} TOOL --args &apos;{"{...}"}&apos;
              </Mono>
              .
            </p>
          </Disclosure>
        </div>
      ) : (
        <div className="grid gap-3">
          <Notice tone="bad">
            <span className="font-medium anywhere">{"Couldn't connect: " + state.error}</span>
            {state.why ? <span className="mt-1 block text-[13px] anywhere">{"Why: " + state.why}</span> : null}
            {state.fix ? <span className="mt-1 block text-[13px] anywhere">{"What to do: " + state.fix}</span> : null}
          </Notice>
          <div className="flex flex-wrap gap-2">
            {onSignIn && wantsSignIn(server, state.fix) ? (
              <Button size="sm" onClick={() => onSignIn(server)}>
                <KeyRoundIcon /> Sign in
              </Button>
            ) : null}
            <Button size="sm" variant="outline" onClick={() => setRun((cur) => ({ attempt: cur.attempt + 1, state: { kind: "running" } }))}>
              Check again
            </Button>
          </div>
        </div>
      )}
    </>
  );
}

// ----- OAuth sign-in -----------------------------------------------------------

// The sign-in `pulse mcp login` runs, from the browser. The hub does the
// discovery and registration, then waits on a thread while the person takes
// the one step: approve in a new tab (code flow) or type a short code on
// the provider's page (device flow). GitHub registers no clients, so for it
// the hub first asks for the id of an OAuth App the person creates.
type SignInState =
  | { kind: "starting" }
  | { kind: "pending"; login: McpLoginState }
  | { kind: "ok"; login: McpLoginState }
  | { kind: "failed"; error: string; why: string; fix: string; login?: McpLoginState }
  | { kind: "needs_client"; login: McpLoginState };

const POLL_MS = 1500;

export function SignInDialog({ server, onOpenChange, onSignedIn }: { server: McpServer | null; onOpenChange: (o: boolean) => void; onSignedIn?: (s: McpServer) => void }) {
  return (
    <Dialog open={!!server} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-xl">{server ? <SignInBody server={server} close={() => onOpenChange(false)} onSignedIn={onSignedIn} /> : null}</DialogContent>
    </Dialog>
  );
}

// Opens `url` in a new tab. Right after a click the browser still counts the
// activation, so this usually works without a blocker; the button stays as
// the fallback either way.
function openTab(url: string): boolean {
  if (typeof window === "undefined" || !url) return false;
  const w = window.open(url, "_blank", "noopener,noreferrer");
  return !!w;
}

const loginPath = (id: string) => "mcp/" + enc(id) + "/login";

function SignInBody({ server, close, onSignedIn }: { server: McpServer; close: () => void; onSignedIn?: (s: McpServer) => void }) {
  const [state, setState] = useState<SignInState>({ kind: "starting" });
  const [attempt, setAttempt] = useState(0);
  const [input, setInput] = useState<McpLoginInput>({});
  const opened = useRef(false);
  // The parent passes a fresh arrow each render; the poll must not restart for it.
  const signedIn = useRef(onSignedIn);
  useEffect(() => {
    signedIn.current = onSignedIn;
  }, [onSignedIn]);
  const terminal = "pulse mcp login " + server.name;

  // One start per attempt: POST, then settle on what the hub said.
  useEffect(() => {
    let cancelled = false;
    opened.current = false;
    writes
      .loginMcp(server.id, input)
      .then((login) => {
        if (cancelled) return;
        if (login.state === "needs_client") setState({ kind: "needs_client", login });
        else if (login.state === "pending") setState({ kind: "pending", login });
        else if (login.state === "ok") setState({ kind: "ok", login });
        else setState({ kind: "failed", error: "The sign-in did not start", why: login.why ?? "", fix: "Try again.", login });
      })
      .catch((e: unknown) => {
        if (cancelled) return;
        setState({ kind: "failed", ...errorText(e) });
      });
    return () => {
      cancelled = true;
    };
    // `input` is set together with `attempt` by the form's submit.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [server.id, attempt]);

  // While pending: open the tab once, then poll until the hub says ok or failed.
  useEffect(() => {
    if (state.kind !== "pending") return;
    const login = state.login;
    if (!opened.current) {
      opened.current = true;
      const at = login.flow === "device" ? login.verification_uri_complete || login.verification_uri || "" : login.url ?? "";
      if (at) openTab(at);
    }
    let cancelled = false;
    const tick = () => {
      api<McpLoginState>(loginPath(server.id))
        .then((next) => {
          if (cancelled) return;
          if (next.state === "ok") {
            setState({ kind: "ok", login: next });
            toast.success("Signed in to " + server.name + ".", { description: "PULSE keeps the tokens on this machine and renews them." });
            refreshMcp();
            signedIn.current?.(next.sign_in ? { ...server, sign_in: next.sign_in } : server);
          } else if (next.state === "failed") {
            setState({ kind: "failed", error: next.cancelled ? "The sign-in was cancelled" : "The sign-in did not finish", why: next.why ?? "", fix: next.cancelled ? "Start it again when you are ready." : "Fix what the why line names and try again.", login: next });
            refreshMcp();
          } else if (next.state === "none") {
            setState({ kind: "failed", error: "The sign-in was dropped", why: "The hub forgot this attempt - it may have restarted.", fix: "Try again." });
          }
        })
        .catch(() => {
          // A missed poll is not a failure; the next one tells.
        });
    };
    const timer = setInterval(tick, POLL_MS);
    return () => {
      cancelled = true;
      clearInterval(timer);
    };
  }, [state, server]);

  const restart = (next: McpLoginInput) => {
    setInput(next);
    setState({ kind: "starting" });
    setAttempt((n) => n + 1);
  };
  const mood = state.kind === "ok" ? "happy" : state.kind === "failed" ? "worried" : state.kind === "needs_client" ? "curious" : state.kind === "pending" ? "waiting" : "busy";

  return (
    <>
      <DialogHeader>
        <DialogTitle className="flex items-center gap-2">
          <KeyRoundIcon className="size-4 text-primary" aria-hidden /> Sign in to {server.name}
        </DialogTitle>
        <DialogDescription>Once. PULSE keeps the tokens on this machine, renews them itself, and agents call the tool from then on.</DialogDescription>
      </DialogHeader>
      <div className="flex items-start gap-4">
        <PulseMascot mood={mood} size={64} decorative className="mt-1 hidden shrink-0 sm:block" />
        <div className="min-w-0 flex-1">
          {state.kind === "starting" ? (
            <div className="flex items-center gap-2 py-6 text-sm text-muted-foreground" role="status" aria-live="polite">
              <Loader2Icon className="size-4 motion-safe:animate-spin" aria-hidden /> Asking {server.name} how to sign in…
            </div>
          ) : state.kind === "pending" ? (
            <PendingStep login={state.login} server={server} />
          ) : state.kind === "ok" ? (
            <div className="grid gap-3">
              <Notice tone="ok">
                <span className="font-medium">Signed in.</span>
                {state.login.note ? <span className="block text-xs text-muted-foreground anywhere">{state.login.note}</span> : null}
              </Notice>
              <p className="text-sm text-muted-foreground">Next, check the connection: PULSE asks the tool what it offers and shows it to agents.</p>
            </div>
          ) : state.kind === "needs_client" ? (
            <ClientForm login={state.login} server={server} initial={input} onStart={restart} />
          ) : (
            <div className="grid gap-3">
              <Notice tone="bad">
                <span className="font-medium anywhere">{state.error}</span>
                {state.why ? <span className="mt-1 block text-[13px] anywhere">{"Why: " + state.why}</span> : null}
                {state.fix ? <span className="mt-1 block text-[13px] anywhere">{"What to do: " + state.fix}</span> : null}
              </Notice>
            </div>
          )}
          {state.kind !== "needs_client" ? (
            <Disclosure summary="Prefer the terminal?" className="mt-3" contentClassName="mt-1.5 text-xs leading-relaxed text-muted-foreground">
              <p>
                The same sign-in runs with <Mono>{terminal}</Mono> on this machine; <Mono>pulse mcp logout {server.name}</Mono> forgets the tokens.
              </p>
            </Disclosure>
          ) : null}
        </div>
      </div>
      <DialogFooter className="gap-2 sm:gap-2">
        {state.kind === "failed" ? (
          <Button size="sm" variant="outline" onClick={() => restart(input)}>
            Try again
          </Button>
        ) : null}
        {state.kind === "ok" ? (
          <Button size="sm" onClick={close}>
            <CheckIcon /> Done
          </Button>
        ) : (
          <Button size="sm" variant="ghost" onClick={close}>
            {state.kind === "pending" ? "Keep waiting in the background" : "Close"}
          </Button>
        )}
      </DialogFooter>
    </>
  );
}

// What the person does now, by flow. The hub keeps waiting even when this
// dialog closes; the card's state line catches up on the next refresh.
function PendingStep({ login, server }: { login: McpLoginState; server: McpServer }) {
  const device = login.flow === "device";
  const at = device ? login.verification_uri ?? "" : login.url ?? "";
  const minutes = Math.max(1, Math.round((login.expires_in ?? 300) / 60));
  return (
    <div className="grid gap-3">
      {device ? (
        <>
          <p className="text-sm leading-relaxed">
            Open the provider&apos;s page and type this code. The page is <span className="font-mono text-[13px]">{hostOf(at)}</span>, where you are already signed in as yourself.
          </p>
          <UserCode code={login.user_code ?? ""} />
          {login.built_in && login.client_label ? (
            <p className="text-xs leading-relaxed text-muted-foreground">
              The page asks you to authorize <strong className="font-medium text-foreground">{login.client_label}</strong>, not PULSE: that is the public app PULSE signs in through, so there is nothing for you to register. The token stays on this machine, in PULSE&apos;s own grant file.
            </p>
          ) : null}
        </>
      ) : (
        <p className="text-sm leading-relaxed">
          A new tab asks you to approve PULSE at <span className="font-mono text-[13px]">{hostOf(at)}</span>. Approve it there and come back; this window notices by itself.
        </p>
      )}
      <div className="flex flex-wrap items-center gap-2">
        <Button size="sm" asChild>
          <a href={device ? login.verification_uri_complete || at : at} target="_blank" rel="noopener noreferrer">
            <ExternalLinkIcon /> {device ? "Open the code page" : "Open the sign-in page"}
          </a>
        </Button>
        <span className="flex items-center gap-1.5 text-xs text-muted-foreground" role="status" aria-live="polite">
          <Loader2Icon className="size-3.5 motion-safe:animate-spin" aria-hidden /> Waiting for {server.name}… up to {minutes} min.
        </span>
      </div>
      {!device && login.redirect_uri ? <p className="text-xs leading-relaxed text-muted-foreground anywhere">The provider sends the approval back to {login.redirect_uri}, a door PULSE opened on this machine just for this sign-in.</p> : null}
    </div>
  );
}

function hostOf(url: string): string {
  try {
    return new URL(url).host || url;
  } catch {
    return url;
  }
}

function UserCode({ code }: { code: string }) {
  const [copied, setCopied] = useState(false);
  const copy = () => {
    void copyText(code)
      .then(() => {
        setCopied(true);
        setTimeout(() => setCopied(false), 1800);
      })
      .catch(() => toast.error("Couldn't copy. Select the code and copy it yourself."));
  };
  return (
    <div className="flex flex-wrap items-center gap-3 rounded-xl border-2 border-dashed border-primary/40 bg-primary/5 px-4 py-3">
      <span className="select-all font-mono text-2xl font-semibold tracking-[0.2em] text-foreground" aria-label={"Code " + code.split("").join(" ")}>
        {code}
      </span>
      <Button size="sm" variant="outline" onClick={copy} aria-label="Copy the code">
        {copied ? <CheckIcon /> : <CopyIcon />} {copied ? "Copied" : "Copy"}
      </Button>
    </div>
  );
}

// The provider registers no clients (GitHub): the person creates one and
// hands over its id. With the device flow that is all; the code flow also
// needs the redirect port the client was registered with and, usually, the
// secret as a reference.
function ClientForm({ login, server, initial, onStart }: { login: McpLoginState; server: McpServer; initial: McpLoginInput; onStart: (input: McpLoginInput) => void }) {
  const deviceOffered = !!login.device_flow;
  const [clientId, setClientId] = useState(initial.client_id ?? "");
  const [flow, setFlow] = useState<McpLoginFlow>(initial.flow === "code" || !deviceOffered ? "code" : "device");
  const [port, setPort] = useState(initial.redirect_port ? String(initial.redirect_port) : login.redirect_port ? String(login.redirect_port) : "");
  const [secretRef, setSecretRef] = useState(initial.client_secret_ref ?? "");
  const [scope, setScope] = useState(initial.scope ?? "");
  const [touched, setTouched] = useState(false);
  const portN = Number(port);
  const portOk = flow === "device" || (port.trim() !== "" && Number.isInteger(portN) && portN >= 1 && portN <= 65535);
  const ready = clientId.trim() !== "" && portOk;
  const docs = login.service_documentation || "";
  const github = /github\.com/.test(login.issuer ?? "") || /github\.com/.test(docs);
  const redirect = "http://127.0.0.1:" + (port.trim() || "PORT") + "/callback";
  const submit = () => {
    setTouched(true);
    if (!ready) return;
    const next: McpLoginInput = { client_id: clientId.trim(), flow };
    if (flow === "code") {
      next.redirect_port = portN;
      if (secretRef.trim()) next.client_secret_ref = secretRef.trim();
    }
    if (scope.trim()) next.scope = scope.trim();
    onStart(next);
  };
  return (
    <div className="grid gap-3">
      <Notice tone="info">
        <span className="font-medium anywhere">{hostOf(login.issuer ?? "") || server.name} does not register apps by itself.</span>
        <span className="mt-1 block text-[13px] anywhere">
          Create one there{docs ? " (its developer settings)" : ""}, then paste its client id here. {deviceOffered ? "With the device flow it needs no secret and no redirect address: you type a short code on the provider's page." : "Register the redirect address shown below with it."}
        </span>
        {docs ? (
          <a href={docs} target="_blank" rel="noopener noreferrer" className="mt-1.5 inline-flex items-center gap-1 text-[13px] font-medium underline underline-offset-2">
            <ExternalLinkIcon className="size-3.5" aria-hidden /> Open {hostOf(docs)}
          </a>
        ) : null}
      </Notice>
      {github ? (
        <Disclosure summary="On GitHub, step by step" contentClassName="mt-1.5 text-xs leading-relaxed text-muted-foreground">
          <ol className="list-decimal space-y-1 pl-4">
            <li>Settings → Developer settings → OAuth Apps → New OAuth App.</li>
            <li>Any name and homepage; for the callback use {deviceOffered && flow === "device" ? "any address (the device flow does not use it)" : <Mono>{redirect}</Mono>}.</li>
            {deviceOffered && flow === "device" ? <li>Tick &quot;Enable Device Flow&quot; and save.</li> : <li>Generate a client secret and keep it as a reference (keychain://, op://, file://), not here.</li>}
            <li>Copy the Client ID (it starts with Iv1. or Ov23) into the field below.</li>
          </ol>
        </Disclosure>
      ) : null}
      <div className="grid gap-1.5">
        <Label htmlFor="mcp-client-id">Client id</Label>
        <Input id="mcp-client-id" value={clientId} onChange={(e) => setClientId(e.target.value)} placeholder={github ? "Iv1.0123456789abcdef" : "the id the provider shows"} spellCheck={false} autoFocus className="font-mono text-[13px]" aria-invalid={touched && !clientId.trim() ? true : undefined} />
        {touched && !clientId.trim() ? <p className="text-xs text-bad">The client id is what the provider gave you when you registered the app.</p> : null}
      </div>
      {deviceOffered ? (
        <div className="grid gap-1.5">
          <Label htmlFor="mcp-flow">How to approve</Label>
          <Select value={flow} onValueChange={(v) => setFlow(v as McpLoginFlow)}>
            <SelectTrigger id="mcp-flow" className="w-full">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="device">Type a code on the provider&apos;s page (no secret, no port)</SelectItem>
              <SelectItem value="code">Approve in a browser tab (needs the redirect port and usually a secret)</SelectItem>
            </SelectContent>
          </Select>
        </div>
      ) : null}
      {flow === "code" ? (
        <>
          <div className="grid gap-1.5">
            <Label htmlFor="mcp-port">Redirect port</Label>
            <Input id="mcp-port" value={port} onChange={(e) => setPort(e.target.value)} inputMode="numeric" placeholder="e.g. 8976" className="font-mono text-[13px]" aria-invalid={touched && !portOk ? true : undefined} />
            <p className="text-xs leading-relaxed text-muted-foreground anywhere">
              The app must be registered with the callback <span className="font-mono text-xs">{redirect}</span>; a fixed port keeps it the same every time.
            </p>
            {touched && !portOk ? <p className="text-xs text-bad">A port between 1 and 65535.</p> : null}
          </div>
          <div className="grid gap-1.5">
            <Label htmlFor="mcp-client-secret">Client secret, as a reference (optional)</Label>
            <Input id="mcp-client-secret" value={secretRef} onChange={(e) => setSecretRef(e.target.value)} placeholder="keychain://pulse/github-oauth-secret" spellCheck={false} className="font-mono text-[13px]" />
            <p className="text-xs leading-relaxed text-muted-foreground">Never the secret itself: op://, keychain:// or file:// pointing at it on this machine.</p>
          </div>
        </>
      ) : null}
      <div className="grid gap-1.5">
        <Label htmlFor="mcp-scope">Scopes (optional)</Label>
        <Input id="mcp-scope" value={scope} onChange={(e) => setScope(e.target.value)} placeholder={(login.scopes_supported ?? []).join(" ") || "what the server suggests"} spellCheck={false} className="font-mono text-[13px]" />
        {login.scopes_supported?.length ? <p className="text-xs leading-relaxed text-muted-foreground anywhere">The server lists: {login.scopes_supported.join(", ")}. Empty takes what it suggests.</p> : null}
      </div>
      <div className="flex flex-wrap items-center gap-2">
        <Button size="sm" onClick={submit} disabled={touched && !ready}>
          <KeyRoundIcon /> Start the sign-in
        </Button>
      </div>
      <Disclosure summary="Prefer the terminal?" contentClassName="mt-1.5 text-xs leading-relaxed text-muted-foreground">
        <p className="anywhere">{login.command || "pulse mcp login " + server.name + " --client-id ID"}</p>
      </Disclosure>
    </div>
  );
}
