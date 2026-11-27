import type { Metadata } from "next";
import Link from "@/components/link";
import {
  ArrowRightIcon,
  ArrowUpRightIcon,
  BookOpenIcon,
  BrainCircuitIcon,
  CalendarClockIcon,
  InboxIcon,
  LaptopIcon,
  PackageIcon,
  PlusIcon,
  PowerIcon,
  ScaleIcon,
  ScrollTextIcon,
  UserCogIcon,
} from "lucide-react";
import type { ReactNode } from "react";
import { Frame } from "@/components/chrome";
import { CodeBlock, CopyLine } from "@/components/code";
import { ConsoleShot } from "@/components/console-shot";
import type { Crop } from "@/components/console-shot";
import { JsonLd } from "@/components/json-ld";
import { LoopDiagram } from "@/components/loop-diagram";
import { GUIDES_DIR, optionalDoc } from "@/lib/docs";
import { FAQS, plainAnswer } from "@/lib/faq";
import { faqJsonLd, pageMetadata, softwareJsonLd } from "@/lib/seo";
import { INSTALL, PAGES, repoFile, SITE } from "@/lib/site";

export const metadata: Metadata = pageMetadata(PAGES.home);

// The page tells one story from top to bottom: what Hive is and how to get it,
// how one task goes (with the console's own pictures), what keeps going between
// tasks, how it improves a project and who stays in charge, how more machines
// join, the questions people ask, and the way in again. Each fact is said once.

/** Four facts a newcomer wants before reading on, one line each. */
const FACTS = [
  { icon: PackageIcon, title: "One binary", text: "One command installs it. The runner and the console stay up as user services." },
  { icon: LaptopIcon, title: "On your machine", text: "Hive has no server of its own. The console listens on 127.0.0.1 only." },
  { icon: BrainCircuitIcon, title: "Your model", text: "GitHub Copilot by default, or OpenAI, Anthropic, Gemini, others, or a local server." },
  { icon: ScaleIcon, title: "MIT licensed", text: "The binary, the console, this site and the docs are one repository." },
];

/**
 * Where a crop starts and how wide it is, in points of the 1440 by 900 picture
 * the console's pictures are taken at. Each number below was measured in the
 * picture: 264 is where the sidebar ends, 433 and 1007 are the edges of the New
 * task dialog, and so on. A retake at another size needs them measured again.
 */
const X = (points: number) => points / 1440;
const Y = (points: number) => points / 900;

/** The picture the page opens with: the console from its top, fading out a little above the window's foot so no row is cut in half. */
const HERO: { shot: string; alt: string; crop: Crop; cropSmall: Crop } = {
  shot: "console-sessions.webp",
  alt: "Sessions, where the console opens: the decisions only you can make, each with its buttons, above the list of sessions",
  crop: { x: 0, y: 0, w: 1, ratio: 1440 / 820 },
  // On a phone: the decisions alone, from the gap above them to the gap under them, so no row is cut across.
  // They are wider than a phone can show at a size to read, so the right side fades.
  cropSmall: { x: X(305), y: Y(153), w: X(400), ratio: 400 / 213, hard: ["left", "top", "bottom"] },
};

type Step = {
  title: string;
  text: string;
  /** The picture of the console that shows the step, with the part of it a column shows and the smaller part a phone shows. */
  shot: string;
  alt: string;
  crop: Crop;
  cropSmall: Crop;
  /** What to read next: a page of the docs by its slug (`how-to/tasks`), or a page of this site by its path (`/harness/`). */
  more: { to: string; label: string };
};

/** One task from the sentence to the merge. Each step shows the page of the console where it happens. */
const STORY: readonly Step[] = [
  {
    title: "You write what you want.",
    text: "A task is a title and a few words in the console. From there it is one conversation: you can type into it at any time to steer.",
    shot: "console-new-task.webp",
    alt: "The New task dialog in the console: a line for what the agents should do, and a box for the details",
    // The dialog, edge to edge, from its top.
    crop: { x: X(433), y: Y(91), w: X(574), ratio: 4 / 3, hard: ["left", "right", "top"] },
    cropSmall: { x: X(433), y: Y(91), w: X(400), ratio: 5 / 4, hard: ["left", "top"] },
    more: { to: `${GUIDES_DIR}/tasks`, label: "Give a task and steer it" },
  },
  {
    title: "The lead cuts it into steps.",
    text: "The project's lead hands each step to an agent in a throwaway clone. Every command it runs shows as it happens.",
    shot: "console-task.webp",
    alt: "A task's conversation in the console: the lead's replies and an agent's commands and edits, one line each",
    // The conversation, from the gap left of it, at the size the other three steps are shown at; the panel beside it
    // starts where the right side fades, so the screen reads as going on.
    crop: { x: X(512), y: Y(106), w: X(540), ratio: 4 / 3, hard: ["left", "top"] },
    cropSmall: { x: X(519), y: Y(106), w: X(407), ratio: 5 / 4, hard: ["left", "right", "top"] },
    more: { to: PAGES.harness.path, label: "How an agent runs" },
  },
  {
    title: "Each step has to pass.",
    text: "A step counts when the project's own check passes. When every step is done, their branches become one pull request.",
    shot: "console-runs.webp",
    alt: "The Runs page of the console: each agent's run, with the project it worked in and the agent that made it",
    // The table of runs, from the gap above its head and the gap right of the sidebar to the gap after the Agent
    // column: every side but the foot stands in a gap, so no word is cut.
    crop: { x: X(296), y: Y(328), w: X(500), ratio: 4 / 3, hard: ["left", "right", "top"] },
    // On a phone: the same table, to the gap after its Project column.
    cropSmall: { x: X(296), y: Y(328), w: X(400), ratio: 5 / 4, hard: ["left", "right", "top"] },
    more: { to: "landing", label: "How work is delivered" },
  },
  {
    title: "A review agent reads it.",
    text: "It merges the pull request when the change is one the agents may decide on, and hands it to you when a person should look.",
    shot: "console-tasks.webp",
    alt: "The Tasks page of the console as a board: a task in progress, and one in the column Asked you",
    // Two columns of the board, In progress and Asked you, cut in the gaps beside them.
    crop: { x: X(573), y: Y(194), w: X(558), ratio: 4 / 3, hard: ["left", "right", "top"] },
    cropSmall: { x: X(852), y: Y(194), w: X(400), ratio: 5 / 4, hard: ["left", "top"] },
    more: { to: `${GUIDES_DIR}/pull-requests`, label: "Review and merge the work" },
  },
];

/** A schedule kept as a file: the one the schedules guide writes, with its brief folded short enough for a phone. */
const AUTOMATION_FILE = `title: Weekly dependency review
every: 1w
at: "09:00"
brief: |
  Update the patch and minor
  versions, run npm test, and
  list the major ones for me.`;

/** Four of the thirteen roles that are built in, with how often each sits and what it looks after, as the role leads guide lists them. */
const ROLES = [
  { role: "docs", sits: "daily", owns: "the guides and the README, kept true to what the code does" },
  { role: "tests", sits: "daily", owns: "what is untested, flaky or slow" },
  { role: "security", sits: "weekly", owns: "dependencies, secrets, permissions and the attack surface" },
  { role: "deps", sits: "weekly", owns: "the lockfile, major versions and deprecations" },
];

/** Who decides what: the three things that stay with the owner. */
const YOURS = [
  {
    icon: InboxIcon,
    title: "Decisions wait for you",
    text: "Hard stops wait for your answer on Sessions, where the console opens: money, what cannot be undone, credentials and publication. Everything else carries on.",
  },
  {
    icon: ScrollTextIcon,
    title: "The rules are yours",
    text: "You write the constitution. Agents may amend parts of it by an informed vote, but never the hard stops or the kill switch. These are rules the agents are held to, not a sandbox: a job runs as your user.",
  },
  {
    icon: PowerIcon,
    title: "One command stops it",
    text: "`hive stop` is the kill switch. The runner starts nothing more until you run `hive start`.",
  },
];

/** Three machines and three jobs: the example the machines docs use, drawn small. */
const FLEET = [
  { name: "this machine", offers: ["macos", "arm64", "xcode", "docker"], job: "needs macos, xcode", note: "the iOS app" },
  { name: "gpu-box", offers: ["linux", "amd64", "gpu"], job: "needs linux, gpu", note: "a model evaluation" },
  { name: "linux-1", offers: ["linux", "amd64"], job: "needs nothing", note: "most jobs" },
];

/** The guides the front page names, by the file's name under docs/how-to/: the path from a first project to a second machine, and where to look when something is wrong. */
const GUIDES = ["first-project", "tasks", "pull-requests", "self-improvement-loop", "add-a-machine", "troubleshooting"];

function Eyebrow({ children }: { children: ReactNode }) {
  return <p className="font-mono text-xs font-medium tracking-[0.18em] text-balance text-teal uppercase">{children}</p>;
}

function Heading({ id, children }: { id: string; children: ReactNode }) {
  return (
    <h2 id={id} className="mt-3 text-3xl leading-[1.12] font-semibold tracking-tight text-balance sm:text-[2.5rem]">
      {children}
    </h2>
  );
}

/** The paragraph under a section's heading. */
function Lead({ children, className = "" }: { children: ReactNode; className?: string }) {
  return <p className={`mt-4 text-lg leading-relaxed text-pretty text-muted-foreground ${className}`}>{children}</p>;
}

/** A sentence with its backticked words set as code. */
function Words({ text }: { text: string }) {
  return (
    <>
      {text.split("`").map((part, i) =>
        i % 2 === 1 ? (
          <code key={i} className="rounded-[0.3em] bg-[var(--code-inline)] px-[0.32em] py-[0.1em] font-mono text-[0.86em] break-words text-foreground">
            {part}
          </code>
        ) : (
          part
        ),
      )}
    </>
  );
}

/** A link at the foot of a block of text: what to read next. */
function More({ href, children }: { href: string; children: ReactNode }) {
  return (
    <Link href={href} className="inline-flex min-h-8 items-center gap-1.5 text-[15px] font-medium text-link hover:underline">
      {children} <ArrowRightIcon className="size-4" aria-hidden="true" />
    </Link>
  );
}

export default function Home() {
  const faqDoc = optionalDoc("faq");
  const concepts = optionalDoc("concepts");
  const troubleshooting = optionalDoc(`${GUIDES_DIR}/troubleshooting`);
  const schedules = optionalDoc(`${GUIDES_DIR}/schedules`);
  const leads = optionalDoc(`${GUIDES_DIR}/role-leads`);
  const guideLinks = GUIDES.map((name) => optionalDoc(`${GUIDES_DIR}/${name}`)).filter((d) => d !== undefined);
  const guides = optionalDoc(GUIDES_DIR);
  return (
    <Frame current="/">
      <JsonLd data={softwareJsonLd()} />
      <JsonLd data={faqJsonLd(FAQS.map((f) => ({ q: f.q, a: plainAnswer(f.a) })))} />

      <section className="night night-field border-b border-border" aria-labelledby="hero-title">
        <div className="mx-auto max-w-6xl px-5 pt-14 sm:px-8 md:pt-20">
          <div className="mx-auto max-w-4xl text-center">
            <Eyebrow>Open source · MIT · macOS, Linux, WSL</Eyebrow>
            <h1 id="hero-title" className="mt-5 text-[2.5rem] leading-[1.06] font-semibold tracking-tight text-balance sm:text-6xl lg:text-[4.25rem]">
              The agent harness that <span className="text-brand">improves your projects</span>, and itself.
            </h1>
            <p className="mx-auto mt-6 max-w-2xl text-lg leading-relaxed text-pretty text-muted-foreground">
              Hive is an open-source harness for coding agents. It runs on your own machine as one binary: it does the tasks you give it, keeps a
              project&apos;s standing work going, and improves every project it works on, under rules you write.
            </p>
            <CopyLine command={INSTALL.script} what="the install command" primary className="mx-auto mt-9 max-w-xl text-left" />
            <div className="mt-5 flex flex-wrap justify-center gap-3">
              <Link href={PAGES.install.path} className="btn btn-quiet">
                Install guide <ArrowRightIcon className="size-4" aria-hidden="true" />
              </Link>
              <a href={SITE.repo} className="btn btn-quiet" rel="noopener noreferrer">
                GitHub <ArrowUpRightIcon className="size-4" aria-hidden="true" />
              </a>
            </div>
            <p className="mt-5 text-sm text-pretty text-muted-foreground">
              It installs <code className="font-mono text-[0.92em] text-foreground">~/.hive/bin/hive</code> on macOS, Linux and Windows inside WSL.{" "}
              <Link href={`${PAGES.install.path}#from-source`} className="link whitespace-nowrap">
                Or build from source
              </Link>
              .
            </p>
          </div>
        </div>
        {/* The product, as wide as the screen: wider than the text column on purpose, so the console's own words can be read. */}
        <div className="mx-auto max-w-[90rem] px-5 pb-14 sm:px-8 md:pb-20">
          <figure className="mt-12 md:mt-14">
            <ConsoleShot
              name={HERO.shot}
              alt={HERO.alt}
              crop={HERO.crop}
              cropSmall={HERO.cropSmall}
              // The same screen at twice the pixels, for a screen that has them: here the picture is drawn nearly as wide as
              // the file is, and on a phone three times as wide as the screen, of which the crop shows a part.
              large="/console-sessions-2x.webp"
              sizes="(min-width: 40rem) min(86rem, 95vw), 330vw"
              bar={
                <>
                  {SITE.console}
                  <span className="max-sm:hidden"> · the console, on your own machine</span>
                </>
              }
              eager
            />
            <figcaption className="mt-3 text-center text-sm text-muted-foreground">
              Sessions, where the console opens: what waits for your answer, and the agents at work. The pictures show the console&apos;s own example data.
            </figcaption>
          </figure>
        </div>
      </section>

      <section aria-labelledby="facts-title" className="border-b border-border">
        <h2 id="facts-title" className="sr-only">
          Hive in four facts
        </h2>
        <ul className="mx-auto grid max-w-6xl gap-x-8 gap-y-7 px-5 py-10 sm:grid-cols-2 sm:px-8 lg:grid-cols-4">
          {FACTS.map((f) => (
            <li key={f.title} className="flex gap-3.5">
              <span className="hex inline-flex h-10 w-[2.2rem] shrink-0 items-center justify-center bg-teal-soft text-teal" aria-hidden="true">
                <f.icon className="size-[1.15rem]" />
              </span>
              {/* The name stands level with the middle of its mark. */}
              <div className="pt-2">
                <p className="font-semibold tracking-tight">{f.title}</p>
                <p className="mt-1 text-[15px] leading-relaxed text-pretty text-muted-foreground">{f.text}</p>
              </div>
            </li>
          ))}
        </ul>
      </section>

      <section id="how" aria-labelledby="story-title" className="scroll-mt-14">
        <div className="mx-auto max-w-6xl px-5 py-20 sm:px-8 lg:py-28">
          <div className="max-w-2xl">
            <Eyebrow>How a task goes</Eyebrow>
            <Heading id="story-title">From a sentence to a{"\u00a0"}merged pull request.</Heading>
            <Lead>You say what you want. Agents do the work where you can watch it, and it comes back as one pull request with a reviewer.</Lead>
          </div>
          {/* Two by two on a wide screen, one under the other on a narrow one. The pictures of a row stand at one height whatever the length of the words above them. */}
          <ol className="mx-auto mt-12 grid max-w-xl grid-cols-1 gap-x-12 gap-y-14 lg:mt-14 lg:max-w-none lg:grid-cols-2 lg:gap-y-16">
            {STORY.map((s, i) => {
              // A page of this site is there by construction; a page of the docs may not be (yet), and then the step has no link.
              const next = s.more.to.startsWith("/") ? s.more.to : optionalDoc(s.more.to)?.path;
              return (
                <li key={s.title} className="flex flex-col">
                  <div className="flex gap-4">
                    <span className="hex mt-0.5 inline-flex h-10 w-[2.2rem] shrink-0 items-center justify-center bg-teal font-mono text-sm font-semibold text-background" aria-hidden="true">
                      {i + 1}
                    </span>
                    <div className="pt-1.5">
                      <h3 className="text-xl font-semibold tracking-tight">{s.title}</h3>
                      <p className="mt-2 leading-relaxed text-pretty text-muted-foreground">{s.text}</p>
                    </div>
                  </div>
                  {/* Pushed to the foot of the words: the links of one row stand level, and so do the pictures under them. */}
                  <div className="mt-auto pt-3">
                    {next && (
                      <p className="pl-[3.2rem]">
                        <More href={next}>{s.more.label}</More>
                      </p>
                    )}
                    <ConsoleShot name={s.shot} alt={s.alt} crop={s.crop} cropSmall={s.cropSmall} className="mt-4" />
                  </div>
                </li>
              );
            })}
          </ol>
        </div>
      </section>

      <section aria-labelledby="standing-title" className="border-y border-border bg-muted/40">
        <div className="mx-auto max-w-6xl px-5 py-20 sm:px-8 lg:py-24">
          <div className="max-w-2xl">
            <Eyebrow>Between tasks</Eyebrow>
            <Heading id="standing-title">A project&apos;s standing work keeps going.</Heading>
            <Lead>Some work is never a task: the weekly review, the check that the site is up, the docs that drift. Hive keeps it going without being asked.</Lead>
          </div>
          <ul className="mt-12 grid grid-cols-1 gap-6 lg:grid-cols-2 lg:gap-12">
            <li className="flex min-w-0 flex-col rounded-card border border-border bg-card p-6 shadow-card">
              <h3 className="flex items-center gap-3 text-xl font-semibold tracking-tight">
                <CalendarClockIcon className="size-5 text-teal" aria-hidden="true" /> Automations
              </h3>
              <p className="mt-3 leading-relaxed text-pretty text-muted-foreground">
                Files under <Words text="`.hive/automations`" /> hold a project&apos;s standing work: schedules, checks, watches and skill runs. Every
                machine that tends the project applies them.
              </p>
              {/* The sample's box grows to the height of the card beside this one, so the two commands stand level. */}
              <div className="mt-5 flex flex-1 flex-col [&_pre]:flex-1 [&>figure]:flex [&>figure]:flex-1 [&>figure]:flex-col">
                <CodeBlock code={AUTOMATION_FILE} lang="yaml" title=".hive/automations/weekly-review.yaml" />
                <CopyLine command="hive manifest check --project my-app" what="the command that checks the file before you commit it" className="mt-4" />
              </div>
              {schedules && (
                <p className="mt-5">
                  <More href={schedules.path}>Run work on a schedule</More>
                </p>
              )}
            </li>
            <li className="flex min-w-0 flex-col rounded-card border border-border bg-card p-6 shadow-card">
              <h3 className="flex items-center gap-3 text-xl font-semibold tracking-tight">
                <UserCogIcon className="size-5 text-teal" aria-hidden="true" /> Role leads
              </h3>
              <p className="mt-3 leading-relaxed text-pretty text-muted-foreground">
                Give one concern of a project an agent of its own that looks after it on a cadence. Thirteen roles are built in, and a word of your own makes
                a new one.
              </p>
              <div className="mt-5 flex flex-1 flex-col">
                {/* The table takes what height the sample beside it has, its rows sharing it. */}
                <dl className="grid flex-1 auto-rows-fr divide-y divide-border rounded-xl border border-border text-[15px]">
                  {ROLES.map((r) => (
                    <div key={r.role} className="grid grid-cols-[5.5rem_minmax(0,1fr)] items-center gap-x-3 px-4 py-2">
                      <dt>
                        <span className="font-mono text-sm font-semibold">{r.role}</span>
                        <span className="block text-[13px] text-muted-foreground">{r.sits}</span>
                      </dt>
                      <dd className="leading-snug text-muted-foreground">{r.owns}</dd>
                    </div>
                  ))}
                </dl>
                <CopyLine command="hive lead add --project my-app --role docs" what="the command that seats a lead for the docs" className="mt-4" />
              </div>
              {leads && (
                <p className="mt-5">
                  <More href={leads.path}>Seat role leads</More>
                </p>
              )}
            </li>
          </ul>
        </div>
      </section>

      <section aria-labelledby="loop-title" className="night night-field">
        <div className="mx-auto max-w-6xl px-5 py-20 sm:px-8 lg:py-28">
          <div className="grid grid-cols-1 gap-12 lg:grid-cols-2 lg:items-center">
            <div>
              <Eyebrow>Self-improvement</Eyebrow>
              <Heading id="loop-title">It gets better at its work, inside rules you write.</Heading>
              <Lead>
                Agents post suggestions and vote on each other&apos;s. When you turn the loop on, it builds the top voted one on an experiment branch, measures it
                and delivers what was kept as a pull request. Hive runs the same loop on its own source.
              </Lead>
              <ul className="mt-7 space-y-4 leading-relaxed">
                {[
                  "A suggestion must name a check that fails on the default branch today and passes with the change.",
                  "Other agents vote. The quorum is two, and an agent's own vote does not count.",
                  "A build is measured against the project's fixtures and against private tests kept outside every repository. Every result is archived, kept or not.",
                ].map((line) => (
                  <li key={line} className="flex gap-3.5">
                    <span className="hex mt-[0.5em] h-2.5 w-[0.55rem] shrink-0 bg-teal" aria-hidden="true" />
                    {line}
                  </li>
                ))}
              </ul>
              <Link href={PAGES.rsi.path} className="btn btn-primary mt-9">
                How the loop works <ArrowRightIcon className="size-4" aria-hidden="true" />
              </Link>
            </div>
            <LoopDiagram className="mx-auto lg:mr-0" />
          </div>

          <div className="mt-16 border-t border-border pt-12 lg:mt-20">
            <h3 className="text-2xl font-semibold tracking-tight">You stay in charge.</h3>
            <ul className="mt-8 grid gap-x-10 gap-y-9 md:grid-cols-3">
              {YOURS.map((y) => (
                <li key={y.title}>
                  <span className="hex inline-flex h-10 w-[2.2rem] items-center justify-center bg-you-soft text-you-ink" aria-hidden="true">
                    <y.icon className="size-[1.15rem]" />
                  </span>
                  <h4 className="mt-4 font-semibold tracking-tight">{y.title}</h4>
                  <p className="mt-2 text-[15px] leading-relaxed text-muted-foreground">
                    <Words text={y.text} />
                  </p>
                </li>
              ))}
            </ul>
          </div>
        </div>
      </section>

      <section aria-labelledby="machines-title">
        <div className="mx-auto max-w-6xl px-5 py-20 sm:px-8 lg:py-24">
          <div className="max-w-2xl">
            <Eyebrow>Your machines</Eyebrow>
            <Heading id="machines-title">
              One machine to start. <span className="sm:block">More when the work needs them.</span>
            </Heading>
            <Lead>
              A project says what its jobs need. A machine says what it offers. Each job goes to a machine that offers everything it needs, and the scarce
              machines are kept for the work only they can do.
            </Lead>
          </div>
          <div className="mt-12 grid grid-cols-1 gap-10 lg:grid-cols-2 lg:gap-12">
            <div className="flex flex-col gap-7">
              <div>
                <h3 className="text-lg font-semibold tracking-tight">Pair a Linux machine over SSH</h3>
                <p className="mt-2 leading-relaxed text-pretty text-muted-foreground">
                  One command, over the SSH login of an admin there. It copies a Linux <Words text="`hive`" /> to the machine, and the machine takes work once it
                  has a model login.
                </p>
                <CopyLine command="hive pair ADMIN@HOST --name NAME" what="the pairing command" className="mt-4" />
              </div>
              <div>
                <h3 className="text-lg font-semibold tracking-tight">Or let a machine dial out</h3>
                <p className="mt-2 leading-relaxed text-pretty text-muted-foreground">
                  A machine with the small <Words text="`hive-hand`" /> program joins with a six-word code and calls out for work. It opens no port of its own. It
                  reaches your console through an SSH tunnel or a proxy that you set up.
                </p>
                <CopyLine command="hive-hand join --brain URL CODE" what="the command a machine joins with" className="mt-4" />
              </div>
              <p className="lg:mt-auto">
                <Link href={PAGES.machines.path} className="btn btn-primary">
                  Pairing machines <ArrowRightIcon className="size-4" aria-hidden="true" />
                </Link>
              </p>
            </div>
            <div className="flex flex-col rounded-card border border-border bg-card shadow-card">
              <p className="border-b border-border px-5 py-3.5 text-sm font-medium text-muted-foreground">An example: three machines, and the job each one takes</p>
              <ul className="grid flex-1 auto-rows-fr divide-y divide-border">
                {FLEET.map((m) => (
                  <li key={m.name} className="flex flex-col justify-center px-5 py-5">
                    <p className="font-mono text-[15px] font-semibold">{m.name}</p>
                    <p className="mt-2 flex flex-wrap gap-1.5">
                      <span className="sr-only"> offers </span>
                      {m.offers.map((w, i) => (
                        <span key={w} className="rounded-full border border-border bg-muted px-2.5 py-0.5 font-mono text-[13px] text-muted-foreground">
                          {w}
                          {i < m.offers.length - 1 && <span className="sr-only">, </span>}
                        </span>
                      ))}
                    </p>
                    <p className="mt-3 flex flex-wrap items-center gap-x-2 gap-y-1 text-[15px]">
                      <ArrowRightIcon className="size-4 text-teal" aria-hidden="true" />
                      <span className="sr-only">. It takes the job that </span>
                      <span className="font-mono text-sm text-teal">{m.job}</span> <span className="text-muted-foreground">({m.note})</span>
                    </p>
                  </li>
                ))}
              </ul>
            </div>
          </div>
        </div>
      </section>

      <section aria-labelledby="faq-title" className="border-t border-border bg-muted/40">
        <div className="mx-auto grid max-w-6xl grid-cols-1 gap-x-12 gap-y-10 px-5 py-20 sm:px-8 lg:grid-cols-[minmax(0,1fr)_minmax(0,2fr)] lg:py-24">
          <div className="lg:sticky lg:top-24 lg:self-start">
            <Eyebrow>Questions</Eyebrow>
            <Heading id="faq-title">Frequently asked questions</Heading>
            <p className="mt-4 text-pretty text-muted-foreground">Short answers. The long ones are in the docs:</p>
            <ul className="mt-4 divide-y divide-border border-y border-border">
              {[concepts, faqDoc, troubleshooting]
                .filter((d) => d !== undefined)
                .map((d) => (
                  <li key={d.path}>
                    <Link href={d.path} className="group flex min-h-12 items-center justify-between gap-4 py-2.5 text-[15px] hover:text-link">
                      {d.label} <ArrowRightIcon className="size-4 shrink-0 text-muted-foreground group-hover:text-link" aria-hidden="true" />
                    </Link>
                  </li>
                ))}
            </ul>
          </div>
          {/* Each question opens its answer; the answers are in the page either way, for a search engine and for a browser's own search. */}
          <div className="divide-y divide-border border-y border-border">
            {FAQS.map((f) => (
              <details key={f.q} className="group">
                <summary className="flex min-h-14 list-none items-center justify-between gap-6 py-4 [&::-webkit-details-marker]:hidden">
                  <h3 className="text-lg font-semibold tracking-tight">{f.q}</h3>
                  <PlusIcon className="size-5 shrink-0 text-muted-foreground transition-transform group-open:rotate-45" aria-hidden="true" />
                </summary>
                <p className="pb-6 leading-relaxed text-pretty text-muted-foreground">
                  <Words text={f.a} />
                  {f.more && (
                    <>
                      {" "}
                      <Link href={f.more.href} className="link whitespace-nowrap">
                        {f.more.label}
                      </Link>
                      .
                    </>
                  )}
                </p>
              </details>
            ))}
          </div>
        </div>
      </section>

      <section aria-labelledby="start-title" className="night night-field border-t border-border">
        <div className="mx-auto max-w-6xl px-5 py-20 sm:px-8 lg:py-24">
          <div className="mx-auto max-w-3xl text-center">
            <Eyebrow>Start</Eyebrow>
            <Heading id="start-title">Install it, then give it a first task.</Heading>
            <CopyLine command={INSTALL.script} what="the install command" className="mx-auto mt-8 max-w-xl text-left" />
            <div className="mt-5 flex flex-wrap justify-center gap-3">
              <Link href={PAGES.install.path} className="btn btn-primary">
                The first run, step by step <ArrowRightIcon className="size-4" aria-hidden="true" />
              </Link>
              <Link href={PAGES.docs.path} className="btn btn-quiet">
                <BookOpenIcon className="size-4" aria-hidden="true" /> Read the docs
              </Link>
            </div>
          </div>

          <div className="mt-16 grid grid-cols-1 gap-12 border-t border-border pt-12 lg:grid-cols-[minmax(0,3fr)_minmax(0,2fr)]">
            {guideLinks.length > 0 && (
              <nav aria-labelledby="guides-title">
                <h3 id="guides-title" className="text-xl font-semibold tracking-tight text-balance">
                  Guides: one job each, from start to finish
                </h3>
                <ul className="mt-5 grid gap-x-8 sm:grid-cols-2">
                  {guideLinks.map((d) => (
                    <li key={d.path} className="border-b border-border">
                      <Link href={d.path} className="group flex min-h-12 items-center justify-between gap-4 py-2.5 text-[15px] hover:text-link">
                        {d.title} <ArrowRightIcon className="size-4 shrink-0 text-muted-foreground group-hover:text-link" aria-hidden="true" />
                      </Link>
                    </li>
                  ))}
                </ul>
                {guides && (
                  <p className="mt-5">
                    <More href={guides.path}>{guides.label}</More>
                  </p>
                )}
              </nav>
            )}
            <div>
              <h3 className="text-xl font-semibold tracking-tight">Open source: read it, run it, change it</h3>
              <p className="mt-4 text-[15px] leading-relaxed text-muted-foreground">
                The licence is{" "}
                <a href={repoFile("LICENSE")} className="link" rel="noopener noreferrer">
                  MIT
                </a>
                , copyright {SITE.year} {SITE.holder}.{" "}
                <a href={repoFile("CONTRIBUTING.md")} className="link" rel="noopener noreferrer">
                  CONTRIBUTING.md
                </a>{" "}
                says how to send a change, and the{" "}
                <Link href="/docs/development/" className="link">
                  development guide
                </Link>{" "}
                how to build and test Hive itself: Rust&nbsp;1.95 or newer and Node.js&nbsp;22 or&nbsp;newer.
              </p>
              <p className="mt-3 text-[15px] leading-relaxed text-muted-foreground">
                Found a problem? Open an{" "}
                <a href={`${SITE.repo}/issues`} className="link" rel="noopener noreferrer">
                  issue on GitHub
                </a>
                . For a vulnerability, follow{" "}
                <a href={repoFile("SECURITY.md")} className="link" rel="noopener noreferrer">
                  SECURITY.md
                </a>
                .
              </p>
              <a href={SITE.repo} className="btn btn-quiet mt-6" rel="noopener noreferrer">
                The repository on GitHub <ArrowUpRightIcon className="size-4" aria-hidden="true" />
              </a>
            </div>
          </div>
        </div>
      </section>
    </Frame>
  );
}
