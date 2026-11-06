"use client";

import Link from "next/link";
import { useEffect, useRef, useState } from "react";
import { ArrowLeftIcon, ArrowRightIcon, CheckCircle2Icon, CircleAlertIcon, CloudIcon, FolderIcon, KeyRoundIcon, LayersIcon, Loader2Icon, PlusIcon, ServerIcon, StethoscopeIcon, XIcon } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { showApiError } from "@/components/pulse/confirm";
import { Disclosure, Notice } from "@/components/pulse/panel";
import { ChoiceCard, WizardBody, WizardDialog, WizardFooter, WizardHeader, type WizardStep } from "@/components/pulse/stepper";
import { Mono } from "@/components/pulse/who";
import { hrefs } from "@/lib/nav";
import { errorText, writes, type AddMachineInput, type Compute, type ComputeCaps, type ComputeRequest, type MachineAdded, type Pairing, type ProjectsPayload } from "@/lib/api";
import { rel } from "@/lib/format";
import { invalidate, useApi } from "@/lib/use-api";
import { cn } from "@/lib/utils";

// Adding a machine from the console, as a few short steps instead of a
// form full of ssh words. The three ways the shell offers:
//
// - Set it up for me (`pulse pair admin@host --name NAME`): the hub reaches
//   an admin account over ssh, creates the pulse user, installs the hands,
//   registers and probes the machine. "Check the machine" first runs the
//   same pairing as a dry run (nothing changes) and shows its report; the
//   real pairing takes minutes and runs on the hub, so the last step
//   watches its report grow and the owner may close the window meanwhile.
// - Already set up (`pulse executor add`): a machine someone prepared by
//   hand - the hands installed and a `pulse` user that answers ssh - is
//   added as drained and checked right away when asked.
// - Rent one (`pulse compute request` + approve): the owner asks the
//   configured cloud provider for a machine for a few hours, like an agent.

export const refreshMachines = () => invalidate((p) => p === "compute" || p === "runner" || p === "org" || p.startsWith("org?") || p.startsWith("projects/"));

const NAME = /^[a-z][a-z0-9-]*$/;
const ADDRESS = /^[A-Za-z0-9._-]+@[A-Za-z0-9._-]+(:\d{1,5})?$/;
const WORD = /^[a-z0-9][a-z0-9._:-]*$/;

function slug(text: string) {
  return text
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 40);
}

// A name from the host part of user@host:port, when it makes a valid one
// ("admin@mac-mini.local" -> "mac-mini"); an IP address gives none.
function nameFromAddress(address: string): string {
  const host = address.split("@")[1]?.split(":")[0] ?? "";
  if (!host || /^[\d.]+$/.test(host)) return "";
  const n = slug(host.split(".")[0] ?? "");
  return NAME.test(n) && n !== "local" && n !== "x" ? n : "";
}

const hint = "text-[12.5px] leading-relaxed text-muted-foreground anywhere";
const bad = "text-[12.5px] leading-relaxed text-bad anywhere";

export function AddMachineButton({ caps, className, size = "sm", variant = "outline" }: { caps?: ComputeCaps; className?: string; size?: "sm" | "default"; variant?: "outline" | "default" }) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <Button type="button" variant={variant} size={size} className={className} onClick={() => setOpen(true)}>
        <PlusIcon />
        Add a machine
      </Button>
      <AddMachineDialog open={open} onOpenChange={setOpen} caps={caps} />
    </>
  );
}

export function AddMachineDialog({ open, onOpenChange, caps }: { open: boolean; onOpenChange: (o: boolean) => void; caps?: ComputeCaps }) {
  // While a request is in flight the window stays open.
  const lock = useRef(false);
  return (
    <WizardDialog open={open} onOpenChange={(o) => !lock.current && onOpenChange(o)}>
      <MachineWizard caps={caps} close={() => onOpenChange(false)} lockRef={lock} />
    </WizardDialog>
  );
}

type Mode = "pair" | "register" | "request";
type Step = "where" | "reach" | "use" | "details" | "review" | "done";

const STEPS: Record<Mode, WizardStep[]> = {
  pair: [
    { id: "where", label: "Where" },
    { id: "reach", label: "Reach it" },
    { id: "use", label: "What it's for" },
    { id: "review", label: "Review" },
  ],
  register: [
    { id: "where", label: "Where" },
    { id: "reach", label: "Reach it" },
    { id: "use", label: "What it's for" },
    { id: "review", label: "Review" },
  ],
  request: [
    { id: "where", label: "Where" },
    { id: "details", label: "Details" },
    { id: "review", label: "Review" },
  ],
};

const MODE_WORDS: Record<Mode, { title: string; icon: React.ComponentType<{ className?: string }>; blurb: string }> = {
  pair: { title: "Set it up for me", icon: KeyRoundIcon, blurb: "A Linux computer you can reach over ssh with an admin account. PULSE installs what it needs and checks it." },
  register: { title: "It's already set up", icon: ServerIcon, blurb: "Someone installed PULSE's hands and a pulse user that answers ssh. PULSE adds it and checks it." },
  request: { title: "Rent one from the cloud", icon: CloudIcon, blurb: "Your cloud provider starts a machine for a few hours, the way an agent asks for one." },
};

// What a project can need that PULSE can't find by itself. The system and
// the chip are always found; the rest are words the owner says it offers.
const OFFERS = ["gpu", "cuda", "android", "xcode", "docker", "chrome"];

// Plain sizes for a rented machine, mapped to each provider's own names.
// "" keeps the size set in Settings.
const SIZES: Record<string, { small: string; medium: string; large: string }> = {
  azure: { small: "Standard_B2s", medium: "Standard_B4ms", large: "Standard_D8s_v5" },
  aws: { small: "t3.medium", medium: "m6i.xlarge", large: "c6i.2xlarge" },
  gcp: { small: "e2-medium", medium: "e2-standard-4", large: "n2-standard-8" },
  multipass: { small: "small", medium: "medium", large: "large" },
};
const SIZE_WORDS: { id: "" | "small" | "medium" | "large"; title: string; note: string }[] = [
  { id: "", title: "As in Settings", note: "The size set for this provider" },
  { id: "small", title: "Small", note: "About 2 cores, 4 GB" },
  { id: "medium", title: "Medium", note: "About 4 cores, 16 GB" },
  { id: "large", title: "Large", note: "About 8 cores, 16-32 GB" },
];
const PROVIDER_NAME: Record<string, string> = { azure: "Azure", aws: "AWS", gcp: "Google Cloud", multipass: "Multipass" };

function MachineWizard({ caps, close, lockRef }: { caps?: ComputeCaps; close: () => void; lockRef: React.RefObject<boolean> }) {
  const provider = caps?.provider || "";
  const [mode, setMode] = useState<Mode>("pair");
  const [step, setStep] = useState<Step>("where");
  const [reach, setReach] = useState(0);
  // Reach it
  const [address, setAddress] = useState("");
  const [nameText, setNameText] = useState("");
  const [nameTouched, setNameTouched] = useState(false);
  // What it's for
  const [onlySome, setOnlySome] = useState(false);
  const [projects, setProjects] = useState<string[]>([]);
  const [offers, setOffers] = useState<string[]>([]);
  const [ownWord, setOwnWord] = useState("");
  const [workdir, setWorkdir] = useState("/srv/pulse");
  const [workers, setWorkers] = useState("1");
  // Register: check right away.
  const [probe, setProbe] = useState(true);
  // Rent: the request.
  const [project, setProject] = useState("");
  const [size, setSize] = useState<"" | "small" | "medium" | "large">("");
  const [exact, setExact] = useState("");
  const [hours, setHours] = useState("4");
  const [reason, setReason] = useState("");
  const [approve, setApprove] = useState(true);
  // The dry run and the real run of a pairing, by their start time.
  const [check, setCheck] = useState<{ started: string; name: string } | null>(null);
  const [run, setRun] = useState<{ started: string; name: string } | null>(null);
  const [checkError, setCheckError] = useState("");
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState<{ kind: "pair" } | { kind: "register"; r: MachineAdded } | { kind: "request"; r: ComputeRequest } | null>(null);
  const body = useRef<HTMLDivElement>(null);

  const projectsQ = useApi<ProjectsPayload>("projects", { refresh: 60000 });
  const active = (projectsQ.data?.projects ?? []).filter((p) => p.status === "active" && p.slug !== "commons");
  const watching = !!(check || run);
  const compute = useApi<Compute>(watching ? "compute" : null, { refresh: 1500 });
  const pairingOf = (w: { started: string; name: string } | null): Pairing | undefined => (w ? compute.data?.pairings?.find((p) => p.name === w.name && p.started === w.started) : undefined);
  const checked = pairingOf(check);
  const running = pairingOf(run);

  const name = nameTouched ? slug(nameText) : nameText ? slug(nameText) : nameFromAddress(address);
  const nameOk = name.length > 0 && NAME.test(name) && name !== "local" && name !== "x";
  const addressOk = ADDRESS.test(address.trim());
  const workdirOk = workdir.trim().startsWith("/") && !workdir.split("/").includes("..");
  const workersN = Number(workers);
  const workersOk = Number.isInteger(workersN) && workersN >= 1 && workersN <= 64;
  const ownOk = !ownWord.trim() || WORD.test(ownWord.trim().toLowerCase());
  const chosenProject = project || (active.length === 1 ? active[0]!.slug : "");
  const hoursN = Number(hours);
  const hoursOk = Number.isInteger(hoursN) && hoursN >= 1 && hoursN <= 72;
  const sizes = SIZES[provider];
  const sizeValue = exact.trim() || (size && sizes ? sizes[size] : "");
  const full = caps ? caps.vms_now >= caps.max_vms : false;
  const providerName = PROVIDER_NAME[provider] ?? provider;

  const steps = STEPS[mode];
  const go = (s: Step) => {
    setStep(s);
    setReach((r) => Math.max(r, steps.findIndex((x) => x.id === s)));
  };
  useEffect(() => {
    body.current?.scrollTo({ top: 0 });
  }, [step]);

  const common = () => ({
    name,
    workdir: workdir.trim() || "/srv/pulse",
    workers: workersOk ? workersN : 1,
    projects: onlySome ? projects.join(",") : "",
    labels: offers.join(","),
  });

  // Check the machine: the pairing as a dry run, nothing on it changes.
  const startCheck = async () => {
    setCheckError("");
    setBusy(true);
    try {
      const r = (await writes.addMachine({ mode: "pair", admin: address.trim(), dry_run: true, ...common() })) as { pairing: Pairing };
      setCheck({ started: r.pairing.started, name: r.pairing.name });
      invalidate("compute");
    } catch (e) {
      const t = errorText(e);
      setCheckError([t.error, t.why, t.fix].filter(Boolean).join(" "));
    } finally {
      setBusy(false);
    }
  };

  const submit = async () => {
    setBusy(true);
    lockRef.current = true;
    try {
      if (mode === "pair") {
        if (check) await writes.dismissPairing(check.name).catch(() => undefined);
        const r = (await writes.addMachine({ mode: "pair", admin: address.trim(), dry_run: false, ...common() })) as { pairing: Pairing };
        setRun({ started: r.pairing.started, name: r.pairing.name });
        setDone({ kind: "pair" });
      } else if (mode === "register") {
        const r = (await writes.addMachine({ mode: "register", ssh: address.trim(), probe, ...common() } as AddMachineInput)) as MachineAdded;
        setDone({ kind: "register", r });
      } else {
        const r = (await writes.addMachine({ mode: "request", project: chosenProject, size: sizeValue, hours: hoursN, reason: reason.trim(), approve })) as { request: ComputeRequest };
        setDone({ kind: "request", r: r.request });
        invalidate((p) => p.startsWith("events"));
      }
      refreshMachines();
      go("done");
    } catch (e) {
      showApiError(e, mode === "request" ? "The machine wasn't requested" : mode === "pair" ? "The machine wasn't paired" : "The machine wasn't added");
    } finally {
      lockRef.current = false;
      setBusy(false);
    }
  };

  const title =
    step === "where"
      ? "Add a machine"
      : step === "reach"
        ? mode === "pair"
          ? "How PULSE reaches it"
          : "Where it is"
        : step === "use"
          ? "What it's for"
          : step === "details"
            ? "The machine you need"
            : step === "review"
              ? "Review"
              : done?.kind === "request"
                ? done.r.status === "approved"
                  ? "A machine is on its way"
                  : "Request sent"
                : done?.kind === "register"
                  ? "Machine added"
                  : running?.status === "ok"
                    ? "Machine added"
                    : running?.status === "failed"
                      ? "The setup didn't finish"
                      : "Setting up " + name;
  const description =
    step === "where"
      ? "Another computer your agents can run on. Pick how you have it."
      : step === "reach"
        ? mode === "pair"
          ? "An account with passwordless sudo that this computer can ssh to. Check the machine first: it shows what PULSE would do, and changes nothing."
          : "The pulse account PULSE reaches the machine with."
        : step === "use"
          ? "Which projects may run on it and what it offers. You can change both later."
          : step === "details"
            ? providerName
              ? "Asks " + providerName + " for one machine. It stops on its own when the hours are up."
              : ""
            : step === "review"
              ? "Nothing has changed yet."
              : "";

  const back = (): Step | null => {
    const i = steps.findIndex((s) => s.id === step);
    return i > 0 ? (steps[i - 1]!.id as Step) : null;
  };

  // The next button and what is missing while it is off.
  let next: React.ReactNode = null;
  let why = "";
  if (step === "where")
    next = (
      <Button onClick={() => go(mode === "request" ? "details" : "reach")} disabled={mode === "request" && !provider}>
        Continue <ArrowRightIcon />
      </Button>
    );
  else if (step === "reach") {
    why = !addressOk ? "Enter the address as user@host." : !nameOk ? "Give it a name." : mode === "pair" && !checked ? "Checking first is recommended." : "";
    next = (
      <Button onClick={() => go("use")} disabled={!addressOk || !nameOk || checked?.status === "running"} variant={mode === "pair" && !checked ? "outline" : "default"}>
        {mode === "pair" && !checked ? "Continue without checking" : "Continue"} <ArrowRightIcon />
      </Button>
    );
  } else if (step === "use") {
    why = !workdirOk ? "The folder must be an absolute path." : !workersOk ? "Jobs at once: 1 to 64." : onlySome && !projects.length ? "Pick at least one project." : "";
    next = (
      <Button onClick={() => go("review")} disabled={!!why}>
        Continue <ArrowRightIcon />
      </Button>
    );
  } else if (step === "details") {
    why = !chosenProject ? "Pick the project." : !hoursOk ? "Hours: 1 to 72." : !reason.trim() ? "Say why in one line." : "";
    next = (
      <Button onClick={() => go("review")} disabled={!!why}>
        Continue <ArrowRightIcon />
      </Button>
    );
  } else if (step === "review")
    next = (
      <Button onClick={() => void submit()} disabled={busy}>
        {busy ? <Loader2Icon className="animate-spin" /> : mode === "request" ? <CloudIcon /> : mode === "pair" ? <KeyRoundIcon /> : <ServerIcon />}
        {mode === "pair" ? "Set it up" : mode === "register" ? "Add it" : approve ? "Start a machine" : "Ask for a machine"}
      </Button>
    );
  else next = <Button onClick={close}>{running?.status === "running" ? "Close, it keeps going" : "Done"}</Button>;

  const prev = back();
  return (
    <>
      <WizardHeader steps={steps} current={step === "done" ? steps[steps.length - 1]!.id : step} reach={reach} done={step === "done"} onGo={(id) => go(id as Step)} title={title} description={description} />
      <WizardBody bodyRef={body}>
        {step === "where" ? (
          <div role="radiogroup" aria-label="How you have it" className="grid gap-2.5">
            {(["pair", "register", "request"] as const).map((m) => (
              <ChoiceCard
                key={m}
                selected={mode === m}
                onSelect={() => {
                  setMode(m);
                  setReach(0);
                }}
                icon={MODE_WORDS[m].icon}
                title={MODE_WORDS[m].title}
                badge={m === "pair" ? <Badge variant="outline">Most common</Badge> : m === "request" && provider ? <Badge variant="outline">{providerName}</Badge> : null}
              >
                {MODE_WORDS[m].blurb}
              </ChoiceCard>
            ))}
            {mode === "request" && !provider ? (
              <Notice tone="info">
                No cloud provider is set up yet.{" "}
                <Link href={hrefs.settings("compute")} className="font-medium underline underline-offset-2" onClick={close}>
                  Choose one in Settings, under Machines
                </Link>{" "}
                (Azure, AWS, Google Cloud or Multipass on this computer), then come back.
              </Notice>
            ) : null}
          </div>
        ) : null}

        {step === "reach" ? (
          <div className="grid gap-4">
            <div className="grid gap-1.5">
              <Label htmlFor="machine-address">{mode === "pair" ? "Admin account" : "PULSE account"}</Label>
              <Input
                id="machine-address"
                value={address}
                onChange={(e) => {
                  setAddress(e.target.value);
                  setCheck(null);
                }}
                placeholder={mode === "pair" ? "e.g. admin@10.0.0.5 or admin@build-box:2222" : "e.g. pulse@gpu-box"}
                spellCheck={false}
                autoCapitalize="off"
                autoFocus
                aria-invalid={(address && !addressOk) || undefined}
                className="font-mono text-[13px]"
              />
              {address && !addressOk ? <p className={bad}>Write it as user@host, with :port when it isn&apos;t 22.</p> : <p className={hint}>{mode === "pair" ? "It needs passwordless sudo, and this computer must be able to ssh to it." : "Usually pulse@ and the machine's name or address."}</p>}
            </div>
            <div className="grid gap-1.5">
              <Label htmlFor="machine-name">Name</Label>
              <Input
                id="machine-name"
                value={nameTouched ? nameText : nameText || name}
                onChange={(e) => {
                  setNameTouched(true);
                  setNameText(e.target.value);
                  setCheck(null);
                }}
                placeholder="e.g. build-box"
                spellCheck={false}
                autoCapitalize="off"
                aria-invalid={(nameTouched && !!nameText && !nameOk) || undefined}
              />
              {nameTouched && nameText && !nameOk ? <p className={bad}>Lowercase words with dashes, starting with a letter, and not &quot;local&quot;.</p> : nameText && name !== nameText.trim() ? <p className={hint}>Saved as {name}.</p> : <p className={hint}>How the machine is listed. Lowercase words with dashes.</p>}
            </div>
            {mode === "pair" ? (
              <MachineCheck pairing={checked} started={!!check} error={checkError} busy={busy} disabled={!addressOk || !nameOk} onCheck={() => void startCheck()} />
            ) : (
              <p className={hint}>PULSE reaches the machine when you add it, so it can take work right away.</p>
            )}
          </div>
        ) : null}

        {step === "use" ? (
          <div className="grid gap-5">
            <fieldset className="grid gap-2">
              <legend className="mb-1 text-sm font-medium">Projects</legend>
              <div role="radiogroup" className="grid gap-2 sm:grid-cols-2">
                <ChoiceCard selected={!onlySome} onSelect={() => setOnlySome(false)} icon={LayersIcon} title="Every project">
                  Any project&apos;s work can run here.
                </ChoiceCard>
                <ChoiceCard selected={onlySome} onSelect={() => setOnlySome(true)} icon={FolderIcon} title="Only some projects">
                  Keep it for the projects you pick.
                </ChoiceCard>
              </div>
              {onlySome ? (
                <div className="grid gap-1.5 rounded-xl border bg-card p-3">
                  {active.length ? (
                    active.map((p) => (
                      <label key={p.slug} className="flex items-center gap-2.5 rounded-md px-1 py-1 text-sm hover:bg-muted/50">
                        <input type="checkbox" className="size-4 accent-primary" checked={projects.includes(p.slug)} onChange={(e) => setProjects((cur) => (e.target.checked ? [...cur, p.slug] : cur.filter((x) => x !== p.slug)))} />
                        <span className="min-w-0 truncate">{p.name || p.slug}</span>
                      </label>
                    ))
                  ) : (
                    <p className={hint}>{projectsQ.data ? "No active project yet." : "Loading the projects…"}</p>
                  )}
                </div>
              ) : null}
            </fieldset>
            <fieldset className="grid gap-2">
              <legend className="mb-1 text-sm font-medium">What it offers</legend>
              <p className={hint}>Pick what a project might need. PULSE finds the system and the chip by itself.</p>
              <div className="flex flex-wrap gap-1.5">
                {[...new Set([...OFFERS, ...offers])].map((w) => {
                  const on = offers.includes(w);
                  return (
                    <button key={w} type="button" aria-pressed={on} onClick={() => setOffers((cur) => (on ? cur.filter((x) => x !== w) : [...cur, w]))} className={cn("inline-flex h-8 items-center gap-1 rounded-full border px-3 text-[13px] transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring", on ? "border-primary bg-primary/10 font-medium text-primary" : "bg-card text-muted-foreground hover:text-foreground")}>
                      {on ? <CheckCircle2Icon className="size-3.5" aria-hidden /> : null}
                      {w}
                    </button>
                  );
                })}
              </div>
              <div className="flex flex-wrap items-start gap-2">
                <div className="grid min-w-48 flex-1 gap-1">
                  <Label htmlFor="machine-own" className="sr-only">
                    Add your own
                  </Label>
                  <Input
                    id="machine-own"
                    value={ownWord}
                    onChange={(e) => setOwnWord(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter" && ownWord.trim() && ownOk) {
                        e.preventDefault();
                        const w = ownWord.trim().toLowerCase();
                        setOffers((cur) => (cur.includes(w) ? cur : [...cur, w]));
                        setOwnWord("");
                      }
                    }}
                    placeholder="Add your own, e.g. large-disk"
                    spellCheck={false}
                    autoCapitalize="off"
                    aria-invalid={!ownOk || undefined}
                  />
                  {!ownOk ? <p className={bad}>Lowercase letters, digits, dots and dashes.</p> : null}
                </div>
                <Button
                  type="button"
                  variant="outline"
                  disabled={!ownWord.trim() || !ownOk}
                  onClick={() => {
                    const w = ownWord.trim().toLowerCase();
                    setOffers((cur) => (cur.includes(w) ? cur : [...cur, w]));
                    setOwnWord("");
                  }}
                >
                  <PlusIcon /> Add
                </Button>
              </div>
            </fieldset>
            <Disclosure summary="More options">
              <div className="grid gap-3 sm:grid-cols-[minmax(0,1fr)_8rem]">
                <div className="grid gap-1.5">
                  <Label htmlFor="machine-workdir">Folder on the machine</Label>
                  <Input id="machine-workdir" value={workdir} onChange={(e) => setWorkdir(e.target.value)} spellCheck={false} autoCapitalize="off" aria-invalid={!workdirOk || undefined} className="font-mono text-[13px]" />
                  {workdirOk ? <p className={hint}>Where its copies of the projects and its runs live.</p> : <p className={bad}>An absolute path, such as /srv/pulse.</p>}
                </div>
                <div className="grid gap-1.5">
                  <Label htmlFor="machine-workers">Jobs at once</Label>
                  <Input id="machine-workers" type="number" min={1} max={64} inputMode="numeric" value={workers} onChange={(e) => setWorkers(e.target.value)} aria-invalid={!workersOk || undefined} />
                  <p className={hint}>A start; each check sets it from the cores and memory.</p>
                </div>
              </div>
            </Disclosure>
          </div>
        ) : null}

        {step === "details" ? (
          <div className="grid gap-4">
            {full ? <Notice tone="warn">Every machine the limits allow is up already. The request waits until one stops, or raise the limit in Settings.</Notice> : null}
            <div className="grid gap-3 sm:grid-cols-[minmax(0,1fr)_8rem]">
              <div className="grid gap-1.5">
                <Label htmlFor="request-project">For project</Label>
                <Select value={chosenProject} onValueChange={setProject} disabled={active.length === 0}>
                  <SelectTrigger id="request-project" className="w-full">
                    <SelectValue placeholder={projectsQ.data ? (active.length ? "Choose a project" : "No active project") : "Loading…"} />
                  </SelectTrigger>
                  <SelectContent>
                    {active.map((p) => (
                      <SelectItem key={p.slug} value={p.slug}>
                        {p.name || p.slug}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                <p className={hint}>Whose work it runs; its hours count there.</p>
              </div>
              <div className="grid gap-1.5">
                <Label htmlFor="request-hours">Hours</Label>
                <Input id="request-hours" type="number" min={1} max={72} inputMode="numeric" value={hours} onChange={(e) => setHours(e.target.value)} aria-invalid={!hoursOk || undefined} />
                <p className={hint}>Then it stops.</p>
              </div>
            </div>
            <fieldset className="grid gap-2">
              <legend className="mb-1 text-sm font-medium">Size</legend>
              <div role="radiogroup" className="grid gap-2 sm:grid-cols-4">
                {SIZE_WORDS.filter((w) => w.id === "" || sizes).map((w) => (
                  <button
                    key={w.id || "default"}
                    type="button"
                    role="radio"
                    aria-checked={size === w.id && !exact.trim()}
                    onClick={() => {
                      setSize(w.id);
                      setExact("");
                    }}
                    className={cn("grid gap-0.5 rounded-xl border bg-card px-3 py-2.5 text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring", size === w.id && !exact.trim() ? "border-primary ring-1 ring-primary" : "hover:bg-muted/40")}
                  >
                    <span className="text-sm font-semibold">{w.title}</span>
                    <span className="text-[12px] leading-snug text-muted-foreground">{w.note}</span>
                  </button>
                ))}
              </div>
              <Disclosure summary="Exact size" open={exact ? true : undefined}>
                <div className="grid gap-1.5">
                  <Label htmlFor="request-exact" className="sr-only">
                    Exact size
                  </Label>
                  <Input id="request-exact" value={exact} onChange={(e) => setExact(e.target.value)} placeholder={sizes ? "e.g. " + sizes.medium : "the provider's size name"} spellCheck={false} autoCapitalize="off" className="font-mono text-[13px]" />
                  <p className={hint}>{providerName + "'s own name for a size. It wins over the choice above."}</p>
                </div>
              </Disclosure>
            </fieldset>
            <div className="grid gap-1.5">
              <Label htmlFor="request-reason">Why</Label>
              <Textarea id="request-reason" value={reason} rows={2} onChange={(e) => setReason(e.target.value)} placeholder="e.g. A long build the laptop can't hold, before Friday's release." />
              <p className={hint}>One line, kept with the request.</p>
            </div>
          </div>
        ) : null}

        {step === "review" ? (
          <div className="grid gap-4">
            <dl className="divide-y rounded-xl border bg-card px-4">
              <Row label="How">{MODE_WORDS[mode].title}</Row>
              {mode === "request" ? (
                <>
                  <Row label="Provider">{providerName}</Row>
                  <Row label="Project">{active.find((p) => p.slug === chosenProject)?.name || chosenProject}</Row>
                  <Row label="Size">{sizeValue ? <Mono>{sizeValue}</Mono> : "As set in Settings"}</Row>
                  <Row label="For">{hoursN === 1 ? "1 hour" : hoursN + " hours"}</Row>
                  <Row label="Why">{reason.trim()}</Row>
                </>
              ) : (
                <>
                  <Row label="Name">{name}</Row>
                  <Row label={mode === "pair" ? "Admin account" : "Address"}>
                    <Mono>{address.trim()}</Mono>
                  </Row>
                  <Row label="Projects">{onlySome ? projects.map((s) => active.find((p) => p.slug === s)?.name || s).join(", ") : "Every project"}</Row>
                  <Row label="Offers">{offers.length ? offers.join(", ") : "Only what PULSE finds itself"}</Row>
                  <Row label="Folder">
                    <Mono>{workdir.trim() || "/srv/pulse"}</Mono>
                  </Row>
                  {mode === "pair" ? <Row label="Checked">{checked?.status === "ok" ? "Yes, it can be set up" : checked?.status === "failed" ? "The check found a problem" : "Not checked"}</Row> : null}
                </>
              )}
            </dl>
            {mode === "pair" ? (
              <p className={hint}>Setting it up takes a few minutes and runs on the hub. You can close this window; the machine shows in the list with its report.</p>
            ) : mode === "register" ? (
              <label className="flex items-start gap-3 rounded-xl border px-3 py-2.5 text-sm">
                <Switch checked={probe} onCheckedChange={setProbe} aria-label="Check it right away" className="mt-0.5" />
                <span className="min-w-0">
                  <span className="font-medium">Check it right away</span>
                  <span className={cn("block", hint)}>Reach the machine now (a minute at most) so it can take work today.</span>
                </span>
              </label>
            ) : (
              <label className="flex items-start gap-3 rounded-xl border px-3 py-2.5 text-sm">
                <Switch checked={approve} onCheckedChange={setApprove} aria-label="Start it right away" className="mt-0.5" />
                <span className="min-w-0">
                  <span className="font-medium">Start it right away</span>
                  <span className={cn("block", hint)}>Approve the request as you make it. Off, it waits for you under Waiting for you.</span>
                </span>
              </label>
            )}
            {mode !== "request" ? (
              <Disclosure summary="Prefer the terminal?">
                <p className={hint}>
                  The same from a terminal on this computer:{" "}
                  <Mono>
                    {mode === "pair"
                      ? "pulse pair " + address.trim() + " --name " + name + (onlySome && projects.length ? " --projects " + projects.join(",") : "")
                      : "pulse executor add " + name + " --ssh " + address.trim() + " --workdir " + (workdir.trim() || "/srv/pulse") + (offers.length ? " --labels " + offers.join(",") : "")}
                  </Mono>
                </p>
              </Disclosure>
            ) : null}
          </div>
        ) : null}

        {step === "done" ? <DoneView done={done} running={running} name={name} onFix={() => go("reach")} /> : null}
      </WizardBody>
      <WizardFooter
        left={
          step !== "done" && prev ? (
            <Button variant="ghost" onClick={() => go(prev)} disabled={busy}>
              <ArrowLeftIcon /> Back
            </Button>
          ) : step === "where" ? (
            <Button variant="ghost" onClick={close}>
              Not now
            </Button>
          ) : null
        }
        why={why}
        right={next}
      />
    </>
  );
}

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="grid gap-0.5 py-2.5 sm:grid-cols-[8rem_minmax(0,1fr)] sm:gap-4">
      <dt className="text-[13px] text-muted-foreground">{label}</dt>
      <dd className="min-w-0 text-sm anywhere">{children}</dd>
    </div>
  );
}

// The dry run: a button, then the hub's report as it grows, then a verdict.
function MachineCheck({ pairing, started, error, busy, disabled, onCheck }: { pairing?: Pairing; started: boolean; error: string; busy: boolean; disabled: boolean; onCheck: () => void }) {
  const running = started && (!pairing || pairing.status === "running");
  return (
    <div className="grid gap-3 rounded-xl border bg-muted/30 p-3.5">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="min-w-0">
          <p className="text-sm font-medium">Check the machine</p>
          <p className={hint}>Signs in, looks around and reports what setting it up would do. Nothing on it changes.</p>
        </div>
        <Button type="button" variant={pairing ? "outline" : "default"} size="sm" onClick={onCheck} disabled={disabled || busy || running}>
          {busy || running ? <Loader2Icon className="animate-spin" /> : <StethoscopeIcon />} {pairing && !running ? "Check again" : running ? "Checking…" : "Check the machine"}
        </Button>
      </div>
      {error ? <Notice tone="bad">{error}</Notice> : null}
      {running ? (
        <p className="flex items-center gap-2 text-[13px] text-muted-foreground" role="status" aria-live="polite">
          <Loader2Icon className="size-3.5 animate-spin" aria-hidden /> {pairing?.lines.length ? pairing.lines[pairing.lines.length - 1] : "Reaching the machine…"}
        </p>
      ) : pairing ? (
        <>
          {pairing.ok ? (
            <Notice tone="ok">It can be set up. Here is what PULSE would do.</Notice>
          ) : (
            <Notice tone="bad">
              The check found a problem: {lastLine(pairing.lines)}. Fix it on the machine, or check the account, then check again.
            </Notice>
          )}
          <Disclosure summary={"The report (" + pairing.lines.length + " lines)"} open={!pairing.ok || undefined}>
            <pre className="max-h-56 overflow-auto rounded-lg bg-card p-2.5 font-mono text-[12px] leading-relaxed whitespace-pre-wrap break-words">{pairing.lines.join("\n")}</pre>
          </Disclosure>
        </>
      ) : null}
    </div>
  );
}

const lastLine = (lines: string[]) => (lines.length ? lines[lines.length - 1]!.replace(/[.\s]+$/, "") : "no report");

function DoneView({ done, running, name, onFix }: { done: { kind: "pair" } | { kind: "register"; r: MachineAdded } | { kind: "request"; r: ComputeRequest } | null; running?: Pairing; name: string; onFix: () => void }) {
  if (!done) return null;
  if (done.kind === "request") {
    const r = done.r;
    return (
      <Result tone="ok" title={r.status === "approved" ? "The runner starts it on its next pass" : "It waits for your yes"}>
        {r.status === "approved" ? "A " + (r.size || "machine") + " for " + r.hours + (r.hours === 1 ? " hour" : " hours") + ". It joins the machines list when it answers, and stops when the hours are up." : "The request is under Waiting for you on the Machines page until you approve it."}
      </Result>
    );
  }
  if (done.kind === "register") {
    const p = done.r.probe;
    return p ? (
      p.ok ? (
        <Result tone="ok" title="It answered">
          {name} is checked and takes work now. {p.note}
        </Result>
      ) : (
        <Result tone="bad" title="It didn't pass its check">
          {name} is listed but takes no work until a check passes. What it said: {p.note}. Fix that on the machine, then use Check on its row.
        </Result>
      )
    ) : (
      <Result tone="ok" title="Added">
        {name} takes work once its first check passes; the runner checks it on its own.
      </Result>
    );
  }
  const status = running?.status ?? "running";
  return (
    <div className="grid gap-3">
      {status === "running" ? (
        <p className="flex items-center gap-2 text-sm" role="status" aria-live="polite">
          <Loader2Icon className="size-4 animate-spin text-primary" aria-hidden /> Setting up {name}. This takes a few minutes; you can close this window.
        </p>
      ) : status === "ok" ? (
        <Result tone="ok" title={name + " is set up"}>
          It takes work once its first check passes. Finished {running?.finished ? rel(running.finished) : "just now"}.
        </Result>
      ) : (
        <Result tone="bad" title="The setup stopped">
          {lastLine(running?.lines ?? [])}. Nothing more happens on its own.{" "}
          <button type="button" className="font-medium underline underline-offset-2" onClick={onFix}>
            Change the details
          </button>{" "}
          and try again.
        </Result>
      )}
      {running?.lines.length ? (
        <Disclosure summary={"The report (" + running.lines.length + " lines)"} open={status !== "ok" || undefined}>
          <pre className="max-h-64 overflow-auto rounded-lg bg-muted/50 p-2.5 font-mono text-[12px] leading-relaxed whitespace-pre-wrap break-words">{running.lines.join("\n")}</pre>
        </Disclosure>
      ) : null}
    </div>
  );
}

function Result({ tone, title, children }: { tone: "ok" | "bad"; title: string; children: React.ReactNode }) {
  return (
    <div className="grid justify-items-center gap-3 py-4 text-center">
      <span className={cn("inline-flex size-12 items-center justify-center rounded-full", tone === "ok" ? "bg-ok/12 text-ok" : "bg-bad/10 text-bad")}>{tone === "ok" ? <CheckCircle2Icon aria-hidden className="size-7" /> : <CircleAlertIcon aria-hidden className="size-7" />}</span>
      <p className="text-lg font-semibold">{title}</p>
      <p className="max-w-md text-sm leading-relaxed text-muted-foreground anywhere">{children}</p>
    </div>
  );
}

// A pairing the hub is running, or finished and not yet dismissed, listed
// above the machines. Running: the last line so far. Finished: the verdict,
// the report unfolded on request, and Dismiss.
export function PairingRow({ p }: { p: Pairing }) {
  const [openReport, setOpenReport] = useState(p.status === "failed");
  const [busy, setBusy] = useState(false);
  const running = p.status === "running";
  const last = p.lines.length ? p.lines[p.lines.length - 1] : "";
  const dismiss = async () => {
    setBusy(true);
    try {
      await writes.dismissPairing(p.name);
      refreshMachines();
    } catch (e) {
      showApiError(e, "The report wasn't dismissed");
    } finally {
      setBusy(false);
    }
  };
  return (
    <li className={cn("min-w-0 rounded-xl border bg-card px-3 py-2.5 text-sm", p.status === "failed" ? "border-bad/40" : running ? "border-primary/40" : "border-ok/40")} aria-live={running ? "polite" : undefined}>
      <div className="flex min-w-0 items-start gap-3">
        {running ? <Loader2Icon className="mt-0.5 size-4 shrink-0 animate-spin text-primary" aria-hidden /> : p.ok ? <CheckCircle2Icon className="mt-0.5 size-4 shrink-0 text-ok" aria-hidden /> : <XIcon className="mt-0.5 size-4 shrink-0 text-bad" aria-hidden />}
        <div className="min-w-0 flex-1">
          <div className="flex min-w-0 flex-wrap items-baseline gap-x-2">
            <span className="font-medium text-foreground anywhere">{p.name}</span>
            <span className="text-[13px] text-muted-foreground anywhere">{(p.dry_run ? "inspecting " : "pairing ") + p.admin}</span>
          </div>
          <p className="mt-0.5 text-[13px] leading-relaxed text-muted-foreground anywhere">
            {running ? (
              <>
                <span className="font-medium text-foreground/80">Working</span>
                {" · started " + rel(p.started) + (last ? " · " + last : "")}
              </>
            ) : p.ok ? (
              <>
                <span className="font-medium text-ok">{p.dry_run ? "Looked" : "Paired"}</span>
                {" · " + rel(p.finished || p.started) + (p.dry_run ? " · nothing changed" : " · it takes work once its probe passes")}
              </>
            ) : (
              <>
                <span className="font-medium text-bad">Did not finish</span>
                {" · " + rel(p.finished || p.started) + (last ? " · " + last : "")}
              </>
            )}
          </p>
          {!running && p.lines.length > 1 ? (
            <button type="button" className="mt-1 text-[12.5px] font-medium text-muted-foreground underline-offset-2 hover:text-foreground hover:underline" onClick={() => setOpenReport((o) => !o)} aria-expanded={openReport}>
              {openReport ? "Hide the report" : "Show the report (" + p.lines.length + " lines)"}
            </button>
          ) : null}
          {openReport && !running ? <pre className="mt-2 max-h-72 overflow-auto rounded-lg bg-muted/50 p-2.5 font-mono text-[12px] leading-relaxed whitespace-pre-wrap break-words text-foreground/90">{p.lines.join("\n")}</pre> : null}
        </div>
        {!running ? (
          <Button type="button" variant="ghost" size="sm" className="-my-1 -mr-1 h-7 shrink-0 px-2 text-[13px]" disabled={busy} onClick={() => void dismiss()}>
            Dismiss
          </Button>
        ) : null}
      </div>
    </li>
  );
}
