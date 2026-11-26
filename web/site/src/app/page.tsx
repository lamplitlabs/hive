import type { Metadata } from "next";
import Link from "@/components/link";
import {
  ArrowRightIcon,
  ArrowUpRightIcon,
  BookOpenIcon,
  CalendarClockIcon,
  DatabaseIcon,
  DownloadIcon,
  GitPullRequestIcon,
  InboxIcon,
  LockIcon,
  MessagesSquareIcon,
  NetworkIcon,
  RefreshCwIcon,
  ScaleIcon,
  ScrollTextIcon,
  UserCogIcon,
} from "lucide-react";
import type { ReactNode } from "react";
import { Frame } from "@/components/chrome";
import { CopyLine } from "@/components/code";
import { ConsoleShot } from "@/components/console-shot";
import { HiveMark } from "@/components/hive-mark";
import { JsonLd } from "@/components/json-ld";
import { LoopDiagram } from "@/components/loop-diagram";
import { Terminal } from "@/components/terminal";
import { GUIDES_DIR, optionalDoc } from "@/lib/docs";
import { FAQS, plainAnswer } from "@/lib/faq";
import { faqJsonLd, pageMetadata, softwareJsonLd } from "@/lib/seo";
import { INSTALL, PAGES, repoFile, SITE } from "@/lib/site";

export const metadata: Metadata = pageMetadata(PAGES.home);

const PILLARS = [
  {
    icon: MessagesSquareIcon,
    title: "Tasks as conversations",
    text: "Write what you want in the console. The project's lead agent cuts it into steps and hands them to agents, and the whole task is one conversation you can steer.",
    href: "/docs/how-to/tasks/",
    more: "Give a task",
  },
  {
    icon: GitPullRequestIcon,
    title: "Pull requests with a reviewer",
    text: "Each step runs in a throwaway clone and must pass the project's test command, when one is set. Finished work becomes a pull request that a review agent merges or hands to you.",
    href: "/docs/how-to/pull-requests/",
    more: "Review and merge",
  },
  {
    icon: CalendarClockIcon,
    title: "Automations",
    text: "A project keeps its standing work as files under .hive/automations: schedules, checks, watches and skill runs. Every machine that tends the project applies the committed files.",
    href: "/docs/how-to/schedules/",
    more: "Run work on a schedule",
  },
  {
    icon: UserCogIcon,
    title: "Role leads",
    text: "Give one concern of a project, such as its docs, its tests or its security, an agent of its own that looks after it on a cadence.",
    href: "/docs/how-to/role-leads/",
    more: "Seat role leads",
  },
  {
    icon: ScaleIcon,
    title: "Self-improvement under a constitution",
    text: "Hive improves each project it works on, and runs the same loop on its own source. Agents post suggestions and vote on them. When you turn the loop on, it builds the top voted one on an experiment branch, measures it and delivers what was kept as a pull request. The constitution is yours: hard stops, paths the loop may never touch, and a kill switch.",
    href: PAGES.rsi.path,
    more: "How the loop works",
  },
  {
    icon: NetworkIcon,
    title: "Many machines",
    text: "hive pair sets up a Linux machine over SSH. Any machine with the small hive-hand program can instead enrol itself with a six-word join code and dial out. Each job goes to a machine that offers what it needs.",
    href: PAGES.machines.path,
    more: "Pairing machines",
  },
];

const TASK = [
  {
    title: "You write what you want.",
    text: "A task is a title and a few words in the console. From there it is a conversation: the lead's replies and every agent's commands appear as they happen, and you can type into it to steer.",
  },
  {
    title: "The lead cuts it into steps.",
    text: "The project's lead agent reads the task and hands each step to an agent. Every step agent works in a throwaway clone of its own.",
  },
  {
    title: "Each step has to pass.",
    text: "A step counts when the project's own check passes: the test command you set for the project. When every step of a task is finished, their branches become one pull request.",
  },
  {
    title: "A review agent reads it.",
    text: "It merges the pull request when the change is one the agents may decide on, and hands it to you when a person should look.",
  },
];

const YOURS = [
  {
    icon: InboxIcon,
    title: "Decisions wait for you",
    text: "Hard stops - money, what cannot be undone, credentials, publication - wait for your answer on Sessions, where the console opens, and every question marked for you waits on its project's For you tab. One exception: a loop that pauses over its own fixtures answers that question itself after a day. Everything else carries on.",
  },
  {
    icon: LockIcon,
    title: "It runs on your machine",
    text: "Hive has no server of its own. The machine talks to the model provider you chose, to your own git host and to package registries for the release dates of dependencies. A job's own commands run with the network open, and a session with an agent may search the web. The console listens on 127.0.0.1 only.",
  },
  {
    icon: GitPullRequestIcon,
    title: "Code arrives as pull requests",
    text: "Each step must pass the project's own check. A review agent merges a pull request or hands it to you.",
  },
  {
    icon: ScrollTextIcon,
    title: "The rules are yours",
    text: "You write the constitution. Agents may amend parts of it by an informed vote. The hard stops and the kill switch are not open to a vote. These are rules the agents are held to, not a sandbox: a job runs as your user.",
  },
  {
    icon: DatabaseIcon,
    title: "Records, not files",
    text: "Decisions, plans, guides and lessons are typed records in the store, each with its history.",
  },
  {
    icon: RefreshCwIcon,
    title: "It keeps itself running",
    text: "User services on macOS (launchd) and Linux (systemd --user), self-update, backups and stall detection.",
  },
];

/** The guides the front page names, by the file's name under docs/how-to/: the path from a first project to a second machine, and where to look when something is wrong. */
const GUIDES = ["first-project", "tasks", "pull-requests", "self-improvement-loop", "add-a-machine", "troubleshooting"];

/** Three more pages of the console, small, under the picture of Sessions. */
const SHOTS = [
  {
    name: "console-tasks.webp",
    title: "Tasks",
    alt: "The Tasks page of the console as a board with the columns Planned, In progress, Asked you and Done",
    text: "Every task you gave, on a board. A card moves to Asked you when the lead needs your decision.",
  },
  {
    name: "console-session.webp",
    title: "Sessions",
    alt: "A session in the console: the agent's steps in the middle and the files it changed on the right",
    text: "Work with one agent turn by turn. Each command and each edit shows as it happens, with the changed files beside it.",
  },
  {
    name: "console-self-improvement.webp",
    title: "Self-improvement",
    alt: "A project's Self-improvement tab in the console, with the counts from suggestion to change",
    text: "How far each suggestion has come, from the vote to the change, and what the loop waits for.",
  },
];

/** Three machines and three jobs: the example the machines docs use, drawn small. */
const FLEET = [
  { name: "this machine", offers: ["macos", "arm64", "xcode", "docker"], job: "needs macos, xcode", note: "the iOS app" },
  { name: "gpu-box", offers: ["linux", "amd64", "gpu"], job: "needs linux, gpu", note: "a model evaluation" },
  { name: "linux-1", offers: ["linux", "amd64"], job: "needs nothing", note: "most jobs" },
];

function Eyebrow({ children }: { children: ReactNode }) {
  return <p className="font-mono text-xs font-medium tracking-[0.18em] text-teal uppercase">{children}</p>;
}

function Heading({ id, children }: { id: string; children: ReactNode }) {
  return (
    <h2 id={id} className="mt-3 text-3xl font-semibold tracking-tight text-balance sm:text-4xl">
      {children}
    </h2>
  );
}

/** An answer with its backticked words set as code. */
function Answer({ text }: { text: string }) {
  return (
    <>
      {text.split("`").map((part, i) =>
        i % 2 === 1 ? (
          <code key={i} className="rounded-[0.3em] bg-[var(--code-inline)] px-[0.32em] py-[0.1em] font-mono text-[0.86em] break-words">
            {part}
          </code>
        ) : (
          part
        ),
      )}
    </>
  );
}

export default function Home() {
  const faqDoc = optionalDoc("faq");
  const concepts = optionalDoc("concepts");
  const tour = optionalDoc(`${GUIDES_DIR}/console-tour`);
  const tasksGuide = optionalDoc(`${GUIDES_DIR}/tasks`);
  const mergeGuide = optionalDoc(`${GUIDES_DIR}/pull-requests`);
  const guideCards = GUIDES.map((name) => optionalDoc(`${GUIDES_DIR}/${name}`)).filter((d) => d !== undefined);
  const guides = optionalDoc(GUIDES_DIR);
  return (
    <Frame current="/">
      <JsonLd data={softwareJsonLd()} />
      <JsonLd data={faqJsonLd(FAQS.map((f) => ({ q: f.q, a: plainAnswer(f.a) })))} />

      <section className="night night-field border-b border-border" aria-labelledby="hero-title">
        <div className="mx-auto grid max-w-6xl gap-12 px-5 pt-14 pb-16 sm:px-8 md:pt-20 md:pb-24 lg:grid-cols-[1.05fr_0.95fr] lg:items-center lg:gap-14">
          <div className="min-w-0">
            <HiveMark className="mark-sweep mark-glow size-16 sm:size-20" draw />
            <p className="mt-8 font-mono text-xs font-medium tracking-[0.18em] text-teal uppercase">Open source · MIT · macOS, Linux and WSL</p>
            <h1 id="hero-title" className="mt-4 text-[2.3rem] leading-[1.08] font-semibold tracking-tight text-balance sm:text-6xl sm:leading-[1.05]">
              The agent harness that <span className="text-brand">improves your projects</span>, and itself.
            </h1>
            <p className="mt-6 max-w-xl text-lg leading-relaxed text-pretty text-muted-foreground">{SITE.sentence}</p>
            <div className="mt-8 max-w-xl">
              <CopyLine command={INSTALL.script} what="the install command" />
              <p className="mt-2.5 text-sm text-muted-foreground">
                macOS and Linux, and Windows inside WSL. It installs <code className="font-mono text-[0.92em] text-foreground">~/.hive/bin/hive</code>.{" "}
                <Link href={`${PAGES.install.path}#from-source`} className="link">
                  Or build from source
                </Link>
                .
              </p>
            </div>
            <div className="mt-8 flex flex-wrap items-center gap-3">
              <Link href={PAGES.install.path} className="btn btn-primary">
                <DownloadIcon className="size-4" aria-hidden="true" /> Install
              </Link>
              <Link href={PAGES.docs.path} className="btn btn-quiet">
                <BookOpenIcon className="size-4" aria-hidden="true" /> Read the docs
              </Link>
              <a href={SITE.repo} className="btn btn-quiet" rel="noopener noreferrer">
                GitHub <ArrowUpRightIcon className="size-4" aria-hidden="true" />
              </a>
            </div>
          </div>
          <div className="min-w-0">
            <Terminal
              title="a first session"
              lines={[
                { cmd: INSTALL.script },
                { cmd: "hive hub" },
                { note: "the console, at http://127.0.0.1:8765" },
                { cmd: "hive harness login" },
                { note: "connect a model: GitHub's device login" },
                { cmd: "hive init --name my-app \\" },
                { cmd: "    --verify 'npm test'", more: true },
                { note: "in your checkout: the project and its check" },
                { cmd: "hive service install" },
                { note: "keep Hive running: the runner and the console as services" },
                { cmd: "hive task new --project my-app \\" },
                { cmd: "    --title 'Rate-limit the public API'", more: true },
                { note: "the lead cuts the steps; the work comes back as a pull request" },
              ]}
            />
            <p className="mt-4 text-sm text-muted-foreground">
              The same steps have buttons in the console.{" "}
              <Link href={PAGES.install.path} className="link">
                The whole first run, step by step
              </Link>
              .
            </p>
          </div>
        </div>
      </section>

      <section aria-labelledby="pillars-title">
        <div className="mx-auto max-w-6xl px-5 py-20 sm:px-8">
          <div className="max-w-2xl">
            <Eyebrow>What it is</Eyebrow>
            <Heading id="pillars-title">One binary for the whole of a project&apos;s work.</Heading>
            <p className="mt-4 text-pretty text-muted-foreground">
              Hive is one binary (<code className="font-mono text-[0.9em]">hive</code>), one SQLite file and one console. Every agent runs on{" "}
              <Link href={PAGES.harness.path} className="link">
                Hive&apos;s own harness
              </Link>
              : a tool loop with limits, deny rules, hooks, MCP servers, skills, helpers and sub-sessions, on GitHub Copilot by default, a key for another
              provider, or a local server.
            </p>
          </div>
          <ul className="mt-12 grid gap-5 sm:grid-cols-2 lg:grid-cols-3">
            {PILLARS.map((p) => (
              <li key={p.title} className="flex flex-col rounded-card border border-border bg-card p-6 shadow-card">
                <span className="hex inline-flex h-11 w-[2.4rem] items-center justify-center bg-teal-soft text-teal">
                  <p.icon className="size-[18px]" aria-hidden="true" />
                </span>
                <h3 className="mt-5 text-lg font-semibold tracking-tight">{p.title}</h3>
                <p className="mt-2 flex-1 text-[15px] leading-relaxed text-muted-foreground">{p.text}</p>
                <Link href={p.href} className="mt-5 inline-flex items-center gap-1.5 text-sm font-medium text-link hover:underline">
                  {p.more} <ArrowRightIcon className="size-4" aria-hidden="true" />
                </Link>
              </li>
            ))}
          </ul>
        </div>
      </section>

      <section aria-labelledby="console-title" className="night night-field border-t border-border">
        <div className="mx-auto max-w-6xl px-5 py-20 sm:px-8">
          <div className="max-w-2xl">
            <Eyebrow>The console</Eyebrow>
            <Heading id="console-title">See what needs you and what is running.</Heading>
            <p className="mt-4 text-pretty text-muted-foreground">
              The console runs on your own machine, at <code className="font-mono text-[0.9em] text-foreground">{SITE.console}</code>. It opens on Sessions: the
              decisions only you can make, anything that stops the agents, and the sessions with them at work. Every button calls what the matching{" "}
              <code className="font-mono text-[0.9em] text-foreground">hive</code> command calls, so a terminal can do the same.
            </p>
          </div>
          <figure className="mt-10">
            <ConsoleShot
              name="console-home.webp"
              large="/console-home-2x.webp"
              sizes="(min-width: 72rem) 68rem, 100vw"
              title="Sessions"
              alt="Sessions, where the console opens: three decisions only you can make, each with its buttons, above the list of sessions"
            />
            <figcaption className="mt-3 text-sm text-muted-foreground">Sessions, on a machine with three projects. The pictures are from the console&apos;s own example data.</figcaption>
          </figure>
          <ul className="mt-10 grid gap-6 sm:grid-cols-3">
            {SHOTS.map((shot) => (
              <li key={shot.name}>
                <ConsoleShot name={shot.name} title={shot.title} alt={shot.alt} zoom />
                <h3 className="mt-4 font-semibold tracking-tight">{shot.title}</h3>
                <p className="mt-1 text-[15px] leading-relaxed text-muted-foreground">{shot.text}</p>
              </li>
            ))}
          </ul>
          {(tour || guides) && (
            <div className="mt-10 flex flex-wrap items-center gap-3">
              {tour && (
                <Link href={tour.path} className="btn btn-primary">
                  Take the tour of the console <ArrowRightIcon className="size-4" aria-hidden="true" />
                </Link>
              )}
              {guides && (
                <Link href={guides.path} className="btn btn-quiet">
                  <BookOpenIcon className="size-4" aria-hidden="true" /> {guides.label}
                </Link>
              )}
            </div>
          )}
        </div>
      </section>

      <section aria-labelledby="task-title" className="border-y border-border bg-muted/40">
        <div className="mx-auto max-w-6xl px-5 py-20 sm:px-8">
          <div className="max-w-2xl">
            <Eyebrow>How a task goes</Eyebrow>
            <Heading id="task-title">From a sentence to a merged pull request.</Heading>
          </div>
          <ol className="mt-12 grid gap-x-8 gap-y-10 sm:grid-cols-2 lg:grid-cols-4">
            {TASK.map((s, i) => (
              <li key={s.title}>
                <span className="hex inline-flex h-11 w-[2.4rem] items-center justify-center bg-primary font-mono text-sm font-semibold text-primary-foreground" aria-hidden="true">
                  {i + 1}
                </span>
                <h3 className="mt-4 font-semibold tracking-tight">{s.title}</h3>
                <p className="mt-2 text-[15px] leading-relaxed text-muted-foreground">{s.text}</p>
              </li>
            ))}
          </ol>
          {(tasksGuide || mergeGuide) && (
            <p className="mt-10 text-sm text-muted-foreground">
              Step by step:{" "}
              {[tasksGuide, mergeGuide]
                .filter((d) => d !== undefined)
                .map((d, i) => (
                  <span key={d.path}>
                    {i > 0 && " and "}
                    <Link href={d.path} className="link">
                      {d.title}
                    </Link>
                  </span>
                ))}
              .
            </p>
          )}
          <p className={`${tasksGuide || mergeGuide ? "mt-2" : "mt-10"} text-sm text-muted-foreground`}>
            More in the docs:{" "}
            <Link href="/docs/console/#tasks" className="link">
              tasks in the console
            </Link>
            ,{" "}
            <Link href="/docs/landing/" className="link">
              delivery
            </Link>{" "}
            and{" "}
            <Link href="/docs/review/" className="link">
              the reviewer
            </Link>
            .
          </p>
        </div>
      </section>

      <section aria-labelledby="loop-title">
        <div className="mx-auto grid max-w-6xl gap-12 px-5 py-20 sm:px-8 lg:grid-cols-2 lg:items-center">
          <div>
            <Eyebrow>The loop</Eyebrow>
            <Heading id="loop-title">It gets better at its work, inside rules you write.</Heading>
            <p className="mt-4 text-pretty text-muted-foreground">
              Between tasks the agents post suggestions and vote on each other&apos;s. This is RSI (recursive self-improvement) with every step written down.
            </p>
            <ul className="mt-6 space-y-3.5 text-[15px] leading-relaxed">
              {[
                "A suggestion must name a check that fails on the default branch today and passes with the change.",
                "Other agents vote. The quorum is two, and an agent's own vote does not count.",
                "When you turn the loop on, it builds the top voted suggestion on an experiment branch and measures it against the project's fixtures and against private tests kept outside every repository.",
                "Every result is archived. What was kept reaches you as a pull request.",
              ].map((line) => (
                <li key={line} className="flex gap-3">
                  <span className="hex mt-[0.45em] h-2.5 w-[0.55rem] shrink-0 bg-teal" aria-hidden="true" />
                  {line}
                </li>
              ))}
            </ul>
            <Link href={PAGES.rsi.path} className="btn btn-primary mt-8">
              How the loop works <ArrowRightIcon className="size-4" aria-hidden="true" />
            </Link>
          </div>
          <LoopDiagram />
        </div>
      </section>

      <section aria-labelledby="machines-title" className="border-y border-border bg-muted/40">
        <div className="mx-auto grid max-w-6xl gap-12 px-5 py-20 sm:px-8 lg:grid-cols-2 lg:items-center">
          <div>
            <Eyebrow>Your machines</Eyebrow>
            <Heading id="machines-title">One machine to start. More when the work needs them.</Heading>
            <p className="mt-4 text-pretty text-muted-foreground">
              A project says what its jobs need. A machine says what it offers. The queue sends each job to a machine that offers everything it needs, and
              keeps the scarce machines for the work only they can do.
            </p>
            <dl className="mt-6 space-y-5 text-[15px] leading-relaxed">
              <div>
                <dt className="font-semibold">Pair over SSH</dt>
                <dd className="mt-1 text-muted-foreground">
                  <code className="font-mono text-[0.9em] text-foreground">hive pair ADMIN@HOST --name NAME</code> sets up a Linux machine over the SSH login of an
                  admin there. It copies a Linux <code className="font-mono text-[0.9em] text-foreground">hive</code> to the machine, and the machine takes work once
                  it has a model login.
                </dd>
              </div>
              <div>
                <dt className="font-semibold">Or let it dial out</dt>
                <dd className="mt-1 text-muted-foreground">
                  A machine with <code className="font-mono text-[0.9em] text-foreground">hive-hand</code> joins with a six-word code and calls out for work.
                  It opens no port of its own. It reaches your console through an SSH tunnel or a proxy that you set up.
                </dd>
              </div>
            </dl>
            <Link href={PAGES.machines.path} className="btn btn-primary mt-8">
              Pairing machines <ArrowRightIcon className="size-4" aria-hidden="true" />
            </Link>
          </div>
          <ul className="space-y-3" aria-label="An example: three machines and the job each one takes">
            {FLEET.map((m) => (
              <li key={m.name} className="rounded-card border border-border bg-card p-4 shadow-card sm:p-5">
                <div className="flex flex-wrap items-center justify-between gap-x-4 gap-y-2">
                  <span className="font-mono text-sm font-semibold">{m.name}</span>
                  <span className="flex flex-wrap gap-1.5">
                    <span className="sr-only"> offers </span>
                    {m.offers.map((w, i) => (
                      <span key={w} className="rounded-full border border-border bg-muted px-2 py-0.5 font-mono text-[11px] text-muted-foreground">
                        {w}
                        {i < m.offers.length - 1 && <span className="sr-only">, </span>}
                      </span>
                    ))}
                  </span>
                </div>
                <p className="mt-3 flex flex-wrap items-center gap-x-2 gap-y-1 border-t border-border pt-3 text-sm">
                  <ArrowRightIcon className="size-4 text-teal" aria-hidden="true" />
                  <span className="sr-only">. It takes the job that </span>
                  <span className="font-mono text-[13px] text-teal">{m.job}</span>{" "}
                  <span className="text-muted-foreground">({m.note})</span>
                </p>
              </li>
            ))}
          </ul>
        </div>
      </section>

      <section aria-labelledby="yours-title">
        <div className="mx-auto max-w-6xl px-5 py-20 sm:px-8">
          <div className="max-w-2xl">
            <Eyebrow>Yours</Eyebrow>
            <Heading id="yours-title">You stay in charge.</Heading>
            <p className="mt-4 text-pretty text-muted-foreground">Hive runs on your machine and answers to you. The agents do the work. What matters waits for your yes or no.</p>
          </div>
          <ul className="mt-12 grid gap-x-8 gap-y-9 sm:grid-cols-2 lg:grid-cols-3">
            {YOURS.map((y) => (
              <li key={y.title} className="flex gap-4">
                <span className="hex inline-flex h-10 w-[2.2rem] shrink-0 items-center justify-center bg-you-soft text-you-ink">
                  <y.icon className="size-4" aria-hidden="true" />
                </span>
                <div>
                  <h3 className="font-semibold tracking-tight">{y.title}</h3>
                  <p className="mt-1 text-[15px] leading-relaxed text-muted-foreground">{y.text}</p>
                </div>
              </li>
            ))}
          </ul>
        </div>
      </section>

      {guideCards.length > 0 && (
        <section aria-labelledby="guides-title" className="border-y border-border bg-muted/40">
          <div className="mx-auto max-w-6xl px-5 py-20 sm:px-8">
            <div className="flex flex-wrap items-end justify-between gap-x-10 gap-y-4">
              <div className="max-w-2xl">
                <Eyebrow>Guides</Eyebrow>
                <Heading id="guides-title">One job each, from start to finish.</Heading>
                <p className="mt-4 text-pretty text-muted-foreground">
                  Each guide says what to click in the console and what to type in a terminal, and ends with how to tell that it worked.
                </p>
              </div>
              {guides && (
                <Link href={guides.path} className="inline-flex items-center gap-1.5 text-sm font-medium text-link hover:underline">
                  {guides.label} <ArrowRightIcon className="size-4" aria-hidden="true" />
                </Link>
              )}
            </div>
            <ul className="mt-10 grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
              {guideCards.map((d) => (
                <li key={d.path}>
                  <Link href={d.path} className="group flex h-full flex-col rounded-card border border-border bg-card p-5 shadow-card hover:border-teal">
                    <h3 className="font-semibold tracking-tight group-hover:text-link">{d.title}</h3>
                    <p className="mt-2 text-sm leading-relaxed text-muted-foreground">{d.description}</p>
                  </Link>
                </li>
              ))}
            </ul>
          </div>
        </section>
      )}

      <section aria-labelledby="open-title" className={guideCards.length > 0 ? undefined : "border-y border-border bg-muted/40"}>
        <div className="mx-auto grid max-w-6xl gap-10 px-5 py-20 sm:px-8 lg:grid-cols-[1fr_1.1fr] lg:items-start">
          <div>
            <Eyebrow>Open source</Eyebrow>
            <Heading id="open-title">MIT licensed. Read it, run it, change it.</Heading>
            <p className="mt-4 text-pretty text-muted-foreground">
              The binary, the console, this site and the documentation live in one repository. The licence is MIT, copyright {SITE.year} {SITE.holder}.
            </p>
            <a href={SITE.repo} className="btn btn-primary mt-8" rel="noopener noreferrer">
              The repository on GitHub <ArrowUpRightIcon className="size-4" aria-hidden="true" />
            </a>
          </div>
          <ul className="grid gap-4 sm:grid-cols-2">
            <li className="rounded-card border border-border bg-card p-5 shadow-card">
              <h3 className="font-semibold tracking-tight">Contribute</h3>
              <p className="mt-2 text-[15px] leading-relaxed text-muted-foreground">
                <a href={repoFile("CONTRIBUTING.md")} className="link" rel="noopener noreferrer">
                  CONTRIBUTING.md
                </a>{" "}
                says how to send a change. The{" "}
                <Link href="/docs/development/" className="link">
                  development guide
                </Link>{" "}
                says how to build and test Hive itself.
              </p>
            </li>
            <li className="rounded-card border border-border bg-card p-5 shadow-card">
              <h3 className="font-semibold tracking-tight">Build it yourself</h3>
              <p className="mt-2 text-[15px] leading-relaxed text-muted-foreground">
                Rust 1.95 or newer and Node.js 22 or newer. The console is exported and embedded in the binary at build time.{" "}
                <Link href={`${PAGES.install.path}#from-source`} className="link">
                  From source
                </Link>
                .
              </p>
            </li>
            <li className="rounded-card border border-border bg-card p-5 shadow-card">
              <h3 className="font-semibold tracking-tight">Report a problem</h3>
              <p className="mt-2 text-[15px] leading-relaxed text-muted-foreground">
                Open an{" "}
                <a href={`${SITE.repo}/issues`} className="link" rel="noopener noreferrer">
                  issue on GitHub
                </a>
                . For a vulnerability, follow{" "}
                <a href={repoFile("SECURITY.md")} className="link" rel="noopener noreferrer">
                  SECURITY.md
                </a>
                .
              </p>
            </li>
            <li className="rounded-card border border-border bg-card p-5 shadow-card">
              <h3 className="font-semibold tracking-tight">The licence</h3>
              <p className="mt-2 text-[15px] leading-relaxed text-muted-foreground">
                <a href={repoFile("LICENSE")} className="link" rel="noopener noreferrer">
                  MIT
                </a>
                : use it, change it and share it, with the copyright and licence notice kept.
              </p>
            </li>
          </ul>
        </div>
      </section>

      <section aria-labelledby="faq-title" className={guideCards.length > 0 ? "border-t border-border bg-muted/40" : undefined}>
        <div className="mx-auto max-w-6xl px-5 py-20 sm:px-8">
          <div className="max-w-2xl">
            <Eyebrow>Questions</Eyebrow>
            <Heading id="faq-title">Frequently asked questions</Heading>
          </div>
          <div className="mt-12 grid gap-x-12 gap-y-10 md:grid-cols-2">
            {FAQS.map((f) => (
              <div key={f.q}>
                <h3 className="text-lg font-semibold tracking-tight">{f.q}</h3>
                <p className="mt-2 text-[15px] leading-relaxed text-muted-foreground">
                  <Answer text={f.a} />
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
              </div>
            ))}
          </div>
          {(faqDoc || concepts) && (
            <p className="mt-12 text-sm text-muted-foreground">
              More answers in the docs:{" "}
              {[concepts, faqDoc]
                .filter((d) => d !== undefined)
                .map((d, i) => (
                  <span key={d.path}>
                    {i > 0 && " and "}
                    <Link href={d.path} className="link">
                      {d.label}
                    </Link>
                  </span>
                ))}
              .
            </p>
          )}
        </div>
      </section>
    </Frame>
  );
}
