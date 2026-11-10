"use client";

import { useEffect, useMemo, useRef, useState } from "react";
import { ArrowLeftIcon, ArrowRightIcon, BugIcon, CheckCircle2Icon, CheckIcon, ClipboardCheckIcon, FileTextIcon, Loader2Icon, PlusIcon, RocketIcon, SparklesIcon, WrenchIcon } from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Textarea } from "@/components/ui/textarea";
import { showApiError } from "@/components/hive/confirm";
import { Markdown } from "@/components/hive/markdown";
import { Notice } from "@/components/hive/panel";
import { ChoiceCard, WizardBody, WizardDialog, WizardFooter, WizardHeader, useWizardLock, type WizardStep } from "@/components/hive/stepper";
import { Mono, StatusBadge, When } from "@/components/hive/who";
import { skillRepoPath, writes, type ProjectsPayload, type Skill, type SkillSuggestion, type SkillsPayload, type ToolkitPayload } from "@/lib/api";
import { plural } from "@/lib/format";
import { EMPTY_SKILL_FORM, formFromSkill, scopeLabel, scopeTitle, skillFormReady, skillInput, skillPatch, validateSkillForm, type SkillForm } from "@/lib/mcp";
import { invalidate, useApi } from "@/lib/use-api";

// A skill is a markdown recipe every worker of a project (or of every
// project) is told about in its brief and reads with `hive skill show`.
// The owner writes one here the way `hive skill add` does; agents add
// their own from the shell.

export const refreshSkills = () => invalidate((p) => p === "skills" || p.startsWith("skills?") || p.startsWith("skills/") || p.startsWith("events"));

const ALL = "__all__";

// `initial` pre-fills a new skill (the toolkit's drafts use it): the
// wizard then opens at the writing step. Editing a saved skill is the
// plain form.
export function SkillDialog({ open, onOpenChange, skill, project, initial }: { open: boolean; onOpenChange: (o: boolean) => void; skill?: Skill | null; project?: string; initial?: Partial<SkillForm> }) {
  const lock = useWizardLock();
  if (skill)
    return (
      <Dialog open={open} onOpenChange={onOpenChange}>
        <DialogContent className="sm:max-w-2xl">{open ? <SkillFormBody skill={skill} project={project ?? ""} close={() => onOpenChange(false)} /> : null}</DialogContent>
      </Dialog>
    );
  return (
    <WizardDialog open={open} onOpenChange={(o) => !lock.current && onOpenChange(o)}>
      <SkillWizard project={project ?? ""} initial={initial} close={() => onOpenChange(false)} lockRef={lock} />
    </WizardDialog>
  );
}

// Starting points for a new skill: the shapes most skills take. Each body
// is a skeleton the owner fills in, never a finished skill.
interface SkillTemplate {
  id: string;
  title: string;
  blurb: string;
  icon: React.ComponentType<{ className?: string }>;
  name: string;
  description: string;
  body: string;
}

const TEMPLATES: SkillTemplate[] = [
  {
    id: "setup",
    title: "Set up the project",
    blurb: "What to install and run before the first change works.",
    icon: WrenchIcon,
    name: "project-setup",
    description: "How to get the project building and its tests passing on a fresh machine",
    body: "## When\nBefore the first change on a fresh clone or machine.\n\n## Steps\n1. Install …\n2. Run …\n\n## Check\nThe command that proves it works, and what it prints.\n",
  },
  {
    id: "release",
    title: "Release checklist",
    blurb: "The steps and checks before a version goes out.",
    icon: RocketIcon,
    name: "release-check",
    description: "How to verify a release before tagging it",
    body: "## When\nBefore tagging a release.\n\n## Steps\n1. …\n\n## Check\nWhat must be true before the tag: tests, changelog, version numbers.\n",
  },
  {
    id: "debug",
    title: "Debugging guide",
    blurb: "Where to look first when a known kind of failure shows up.",
    icon: BugIcon,
    name: "debug-guide",
    description: "Where to look first when the build or the tests fail in a known way",
    body: "## Symptoms\nWhat the failure looks like.\n\n## Likely causes\n- …\n\n## How to confirm\nThe command or log that tells the causes apart.\n\n## Fix\n…\n",
  },
  {
    id: "review",
    title: "Review checklist",
    blurb: "What every change must get right before it is done.",
    icon: ClipboardCheckIcon,
    name: "review-checklist",
    description: "What to check in a change before calling it done",
    body: "## Before calling it done\n- [ ] Tests cover the change\n- [ ] …\n\n## Never\n- …\n",
  },
];

type SkillStep = "start" | "write" | "check" | "done";
const SKILL_STEPS: WizardStep[] = [
  { id: "start", label: "Start from" },
  { id: "write", label: "Write" },
  { id: "check", label: "Check" },
];

function SkillWizard({ project, initial, close, lockRef }: { project: string; initial?: Partial<SkillForm>; close: () => void; lockRef: React.RefObject<boolean> }) {
  const { data: projects } = useApi<ProjectsPayload>("projects", { refresh: false });
  const toolkit = useApi<ToolkitPayload>("toolkit", { refresh: false });
  const existing = useApi<SkillsPayload>("skills", { refresh: false });
  const [step, setStep] = useState<SkillStep>(initial ? "write" : "start");
  const [reach, setReach] = useState(initial ? 1 : 0);
  const [choice, setChoice] = useState(initial ? "initial" : "");
  const [f, setF] = useState<SkillForm>(() => ({ ...EMPTY_SKILL_FORM, project, ...initial }));
  const [busy, setBusy] = useState(false);
  const [touched, setTouched] = useState(false);
  const [added, setAdded] = useState<Skill | null>(null);
  const body = useRef<HTMLDivElement>(null);
  const set = <K extends keyof SkillForm>(k: K, v: SkillForm[K]) => setF((cur) => ({ ...cur, [k]: v }));
  const errors = validateSkillForm(f);
  const slugs = (projects?.projects ?? []).filter((p) => p.status === "active").map((p) => p.slug);
  const taken = new Set((existing.data?.skills ?? []).map((k) => k.name));
  const drafts: SkillSuggestion[] = useMemo(() => {
    const d = toolkit.data;
    if (!d) return [];
    return (d.projects ?? [{ project: d.project, tools: [], skills: d.skills ?? [] }]).flatMap((g) => g.skills).filter((k) => !taken.has(k.name));
    // `taken` follows the skills list.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [toolkit.data, existing.data]);

  useEffect(() => {
    body.current?.scrollTo({ top: 0 });
  }, [step]);
  const go = (s: SkillStep) => {
    setStep(s);
    setReach((r) => Math.max(r, SKILL_STEPS.findIndex((x) => x.id === s)));
  };
  const pick = (key: string) => {
    setChoice(key);
    setTouched(false);
    const t = TEMPLATES.find((x) => "t:" + x.id === key);
    const d = drafts.find((x) => "d:" + x.project + ":" + x.name === key);
    if (t) setF({ name: t.name, project, description: t.description, body: t.body });
    else if (d) setF({ name: d.name, project: d.project, description: d.description, body: d.body });
    else setF({ ...EMPTY_SKILL_FORM, project });
  };
  const submit = async () => {
    setTouched(true);
    if (errors.length) return;
    setBusy(true);
    lockRef.current = true;
    try {
      const r = await writes.addSkill(skillInput(f));
      setAdded(r.skill);
      refreshSkills();
      go("done");
    } catch (e) {
      showApiError(e, "The skill wasn't added");
    } finally {
      lockRef.current = false;
      setBusy(false);
    }
  };
  const title = step === "start" ? "Add a skill" : step === "write" ? "Write the skill" : step === "check" ? "How agents will see it" : (added?.name ?? f.name) + " is added";
  const description =
    step === "start"
      ? "A set of instructions agents read before work it covers. Start from a shape, or from a draft Hive wrote from your projects."
      : step === "write"
        ? "Fill in the parts that are yours. Write it in Markdown."
        : step === "check"
          ? "The line on top is what every agent sees in its brief; it reads the rest before the work."
          : "";
  const why = step === "start" ? (!choice ? "Pick a starting point." : "") : step === "write" ? (!skillFormReady(f) ? "It needs a name, a one-line description and the instructions." : touched && errors.length ? "Fix what's listed above." : "") : "";

  return (
    <>
      <WizardHeader steps={SKILL_STEPS} current={step === "done" ? "check" : step} reach={reach} done={step === "done"} onGo={(id) => go(id as SkillStep)} title={title} description={description} />
      <WizardBody bodyRef={body}>
        {step === "start" ? (
          <div className="grid gap-5">
            {drafts.length ? (
              <section className="grid gap-2" aria-label="Drafted from your projects">
                <div className="flex items-baseline gap-2">
                  <SparklesIcon aria-hidden className="size-4 translate-y-0.5 text-primary" />
                  <h3 className="text-sm font-semibold">Drafted from your projects</h3>
                </div>
                <div role="radiogroup" aria-label="Drafted from your projects" className="grid gap-2 sm:grid-cols-2">
                  {drafts.map((d) => (
                    <ChoiceCard key={d.project + d.name} selected={choice === "d:" + d.project + ":" + d.name} onSelect={() => pick("d:" + d.project + ":" + d.name)} icon={SparklesIcon} title={d.name}>
                      {"For " + d.project + ". " + d.description}
                    </ChoiceCard>
                  ))}
                </div>
              </section>
            ) : null}
            <section className="grid gap-2" aria-label="Start from a shape">
              <h3 className="text-sm font-semibold">Start from a shape</h3>
              <div role="radiogroup" aria-label="Start from a shape" className="grid gap-2 sm:grid-cols-2">
                {TEMPLATES.map((t) => (
                  <ChoiceCard key={t.id} selected={choice === "t:" + t.id} onSelect={() => pick("t:" + t.id)} icon={t.icon} title={t.title}>
                    {t.blurb}
                  </ChoiceCard>
                ))}
                <ChoiceCard selected={choice === "blank"} onSelect={() => pick("blank")} icon={FileTextIcon} title="Blank">
                  Write it from scratch.
                </ChoiceCard>
              </div>
            </section>
          </div>
        ) : null}

        {step === "write" ? (
          <div className="grid gap-4">
            <SkillFields f={f} set={set} slugs={slugs} />
            {touched && errors.length ? (
              <Notice tone="warn">
                <ul className="list-disc pl-4">
                  {errors.map((e) => (
                    <li key={e}>{e}</li>
                  ))}
                </ul>
              </Notice>
            ) : null}
          </div>
        ) : null}

        {step === "check" ? (
          <div className="grid gap-3">
            <div className="rounded-lg border bg-muted/40 px-4 py-3 text-sm">
              <span className="font-mono text-[13px] font-semibold">{f.name.trim()}</span>
              <span className="text-muted-foreground"> · {scopeLabel(f.project)} · </span>
              {f.description.trim()}
            </div>
            <div className="max-h-[45dvh] overflow-y-auto rounded-lg border px-4 py-3">
              <Markdown source={f.body} />
            </div>
            {/\u2026|\.\.\./.test(f.body) ? <Notice tone="warn">The instructions still have gaps (…) from the template. Go back and fill them in, or agents will read them as they are.</Notice> : null}
          </div>
        ) : null}

        {step === "done" && added ? (
          <div className="grid justify-items-center gap-3 py-6 text-center">
            <span className="inline-flex size-12 items-center justify-center rounded-full bg-ok/12 text-ok">
              <CheckCircle2Icon aria-hidden className="size-7" />
            </span>
            <p className="text-xl font-semibold">Agents know about it now</p>
            <p className="max-w-md text-sm text-muted-foreground">{"Agents of " + scopeLabel(added.project) + " see " + added.name + " in their brief from their next job, and read it before work it covers."}</p>
          </div>
        ) : null}
      </WizardBody>
      <WizardFooter
        left={
          step === "start" ? (
            <Button variant="ghost" onClick={close}>
              Not now
            </Button>
          ) : step !== "done" ? (
            <Button variant="ghost" onClick={() => go(step === "check" ? "write" : "start")} disabled={busy}>
              <ArrowLeftIcon /> Back
            </Button>
          ) : null
        }
        why={why}
        right={
          step === "start" ? (
            <Button onClick={() => go("write")} disabled={!choice}>
              Continue <ArrowRightIcon />
            </Button>
          ) : step === "write" ? (
            <Button
              onClick={() => {
                setTouched(true);
                if (!errors.length) go("check");
              }}
              disabled={!skillFormReady(f) || (touched && errors.length > 0)}
            >
              Continue <ArrowRightIcon />
            </Button>
          ) : step === "check" ? (
            <Button onClick={() => void submit()} disabled={busy}>
              {busy ? <Loader2Icon className="animate-spin" /> : <PlusIcon />} Add the skill
            </Button>
          ) : (
            <Button onClick={close}>Done</Button>
          )
        }
      />
    </>
  );
}

// The fields of a skill, for the wizard and the edit form.
function SkillFields({ f, set, slugs }: { f: SkillForm; set: <K extends keyof SkillForm>(k: K, v: SkillForm[K]) => void; slugs: string[] }) {
  const knownProject = !f.project || slugs.includes(f.project);
  return (
    <div className="grid gap-4">
      <div className="grid gap-3 sm:grid-cols-2">
        <div className="grid content-start gap-1.5">
          <Label htmlFor="skill-name">Name</Label>
          <Input id="skill-name" value={f.name} onChange={(e) => set("name", e.target.value)} placeholder="e.g. release-check" maxLength={64} spellCheck={false} className="font-mono text-[13px]" />
          <p className="text-xs text-muted-foreground">Short and lowercase. Unique across projects.</p>
        </div>
        <div className="grid content-start gap-1.5">
          <Label htmlFor="skill-project">Who can use it</Label>
          <Select value={f.project || ALL} onValueChange={(v) => set("project", v === ALL ? "" : v)}>
            <SelectTrigger id="skill-project" className="w-full">
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
          <p className="text-xs text-muted-foreground">Agents of every project, or of one.</p>
        </div>
      </div>
      <div className="grid content-start gap-1.5">
        <Label htmlFor="skill-description">When to use it</Label>
        <Input id="skill-description" value={f.description} onChange={(e) => set("description", e.target.value)} placeholder="e.g. How to verify a release before tagging it" maxLength={200} />
        <p className="text-xs text-muted-foreground">One line. Agents see it in their brief, so say what the skill covers and when to read it.</p>
      </div>
      <div className="grid content-start gap-1.5">
        <div className="flex items-center justify-between">
          <Label htmlFor="skill-body">Instructions (Markdown)</Label>
          <span className="text-xs text-muted-foreground tnum">{plural(f.body.length, "character")}</span>
        </div>
        <Tabs defaultValue="write">
          <TabsList className="h-7">
            <TabsTrigger value="write" className="h-6 px-2 text-xs">
              Write
            </TabsTrigger>
            <TabsTrigger value="preview" className="h-6 px-2 text-xs">
              Preview
            </TabsTrigger>
          </TabsList>
          <TabsContent value="write">
            <Textarea id="skill-body" value={f.body} onChange={(e) => set("body", e.target.value)} rows={12} className="min-h-48 font-mono text-[13px]" placeholder={"## When\nBefore tagging a release.\n\n## Steps\n1. ...\n\n## Check\nThe command that proves it worked."} />
          </TabsContent>
          <TabsContent value="preview">
            <div className="min-h-48 rounded-md border p-3">
              <Markdown source={f.body} />
            </div>
          </TabsContent>
        </Tabs>
        <p className="text-xs text-muted-foreground">Never paste a secret here: every agent can read skills. Say where it is kept instead (a reference or an environment variable).</p>
      </div>
    </div>
  );
}

function SkillFormBody({ skill, project, close }: { skill: Skill; project: string; close: () => void }) {
  const { data: projects } = useApi<ProjectsPayload>("projects", { refresh: false });
  const [f, setF] = useState<SkillForm>(() => (skill ? formFromSkill(skill) : { ...EMPTY_SKILL_FORM, project }));
  const [busy, setBusy] = useState(false);
  const [touched, setTouched] = useState(false);
  const set = <K extends keyof SkillForm>(k: K, v: SkillForm[K]) => setF((cur) => ({ ...cur, [k]: v }));
  const errors = validateSkillForm(f);
  const slugs = (projects?.projects ?? []).filter((p) => p.status === "active").map((p) => p.slug);

  const submit = async () => {
    setTouched(true);
    if (errors.length) return;
    setBusy(true);
    const id = toast.loading("Saving " + skill.name + "…");
    try {
      const patch = skillPatch(skill, f);
      if (!Object.keys(patch).length) {
        toast.dismiss(id);
        toast.message("Nothing changed.");
        close();
        return;
      }
      const r = await writes.updateSkill(skill.id, patch);
      toast.success(r.skill.name + " saved.", { id });
      refreshSkills();
      close();
    } catch (e) {
      toast.dismiss(id);
      showApiError(e, "The skill wasn't saved");
    } finally {
      setBusy(false);
    }
  };

  return (
    <>
      <DialogHeader>
        <DialogTitle>{"Edit " + skill.name}</DialogTitle>
        <DialogDescription>A set of instructions agents read before work it covers: a setup, a debugging path, a check. Write it in Markdown.</DialogDescription>
      </DialogHeader>
      <SkillFields f={f} set={set} slugs={slugs} />
      {touched && errors.length ? (
        <Notice tone="warn">
          <ul className="list-disc pl-4">
            {errors.map((e) => (
              <li key={e}>{e}</li>
            ))}
          </ul>
        </Notice>
      ) : null}
      <DialogFooter className="items-center gap-2">
        <Button variant="ghost" onClick={close} disabled={busy}>
          Cancel
        </Button>
        <Button onClick={() => void submit()} disabled={busy || !skillFormReady(f) || (touched && errors.length > 0)}>
          {busy ? <Loader2Icon className="animate-spin" /> : <CheckIcon />}
          Save
        </Button>
      </DialogFooter>
    </>
  );
}

// The body as an agent reads it, with the facts the brief does not carry.
export function SkillViewDialog({ skill, onOpenChange, onEdit }: { skill: Skill | null; onOpenChange: (o: boolean) => void; onEdit?: (k: Skill) => void }) {
  return (
    <Dialog open={!!skill} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-2xl">
        {skill ? (
          <>
            <DialogHeader>
              <DialogTitle className="flex flex-wrap items-center gap-2">
                <span className="anywhere">{skill.name}</span>
                <StatusBadge status={skill.status} />
              </DialogTitle>
              <DialogDescription className="anywhere">{skill.description}</DialogDescription>
            </DialogHeader>
            <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-[13px] text-muted-foreground">
              <span>{scopeTitle(skill.project)}</span>
              <span>{skill.uses ? "Read " + (skill.uses === 1 ? "once" : skill.uses + " times") : "Not read yet"}</span>
              <span>
                {(skillRepoPath(skill) ? "Kept by the repository" : "Added by " + (skill.created_by === "human" || skill.created_by === "owner" ? "you" : skill.created_by)) + ", updated "}
                <When d={skill.updated_at} />
              </span>
            </div>
            <div className="rounded-lg border px-4 py-3">
              <Markdown source={skill.body} />
            </div>
            <p className="text-xs text-muted-foreground">
              Agents read it with <Mono>hive skill show {skill.name}</Mono>; each read is counted.
              {skillRepoPath(skill) ? (
                <>
                  {" The text lives in "}
                  <Mono>{skillRepoPath(skill)}</Mono>
                  {" on the project's default branch: edit it there and commit; Hive loads the new text by itself. Turning it off here lasts until the next commit that changes the project's skill folders or Hive file; to drop it, delete the folder."}
                </>
              ) : null}
            </p>
            <DialogFooter>
              {onEdit ? (
                <Button variant="outline" size="sm" onClick={() => onEdit(skill)} aria-label={"Edit " + skill.name}>
                  Edit
                </Button>
              ) : null}
              <Button size="sm" onClick={() => onOpenChange(false)}>
                Close
              </Button>
            </DialogFooter>
          </>
        ) : null}
      </DialogContent>
    </Dialog>
  );
}
