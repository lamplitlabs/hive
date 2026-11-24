import type { Metadata } from "next";
import Link from "@/components/link";
import { Frame } from "@/components/chrome";
import { CodeBlock, CopyLine } from "@/components/code";
import { Article, H2, PageCrumbs, PageHero } from "@/components/page-hero";
import { GUIDES_DIR, optionalDoc } from "@/lib/docs";
import { pageMetadata } from "@/lib/seo";
import { INSTALL, PAGES, repoFile } from "@/lib/site";

export const metadata: Metadata = pageMetadata(PAGES.install);

const SECTIONS = [
  { id: "need", title: "What you need" },
  { id: "install", title: "1. Install" },
  { id: "console", title: "2. Open the console" },
  { id: "model", title: "3. Connect a model" },
  { id: "project", title: "4. Add a project" },
  { id: "runner", title: "5. Keep Hive running" },
  { id: "task", title: "6. Give the lead a task" },
  { id: "decide", title: "7. Say yes or no" },
  { id: "next", title: "Where to go next" },
];

export default function InstallPage() {
  const commands = optionalDoc("commands");
  const settings = optionalDoc("settings");
  const guide = (name: string) => optionalDoc(`${GUIDES_DIR}/${name}`);
  const guides = optionalDoc(GUIDES_DIR);
  const firstProject = guide("first-project");
  const daily = [guide("tasks"), guide("pull-requests"), guide("what-needs-you")].filter((d) => d !== undefined);
  const trouble = guide("troubleshooting");
  return (
    <Frame current={PAGES.install.path}>
      <PageCrumbs page={PAGES.install} />
      <PageHero
        eyebrow="Install"
        title="Install Hive and give it a first task."
        lead="One command installs the binary. The console opens in your browser, on your own machine. Then you connect a model, add a project, start the runner and give the project's lead agent its first task."
      >
        <div className="mt-8 max-w-xl">
          <CopyLine command={INSTALL.script} what="the install command" />
        </div>
      </PageHero>

      <Article sections={SECTIONS}>
        <H2 id="need">What you need</H2>
        <ul>
          <li>
            macOS or Linux. On Windows, Hive runs inside WSL, the Linux that Windows runs:{" "}
            <Link href="/docs/how-to/windows/">Run Hive on Windows</Link> has the steps. It does not run on Windows itself yet.
          </li>
          <li>A model: a GitHub Copilot seat, an API key for another model provider, or a local model server.</li>
          <li>
            To deliver work as pull requests: <code>git</code>, and the GitHub CLI (<code>gh</code>) logged in as a GitHub user who can see the repository.
            Hive opens, reads and merges pull requests through <code>gh</code>.
          </li>
          <li>There is no other agent tool to install or keep logged in.</li>
        </ul>

        <H2 id="install">1. Install</H2>
        <CodeBlock code={INSTALL.blocks.script} title="macOS, Linux and WSL" copy={INSTALL.script} copyWhat="the install command" />
        <p>
          The installer is one shell script. It is{" "}
          <a href={repoFile("scripts/install.sh")} rel="noopener noreferrer">
            <code>scripts/install.sh</code>
          </a>{" "}
          in the repository, and this site serves the same file, so you can read it before you run it. Then put the binary on your path, check it and
          start the console:
        </p>
        <CodeBlock code={INSTALL.blocks.after} title="Then" />
        <p>
          The first <code>hive hub</code> on a machine makes the store, <code>~/.hive/hive.sqlite</code>, and says so in one line.{" "}
          <code>hive init --store-only</code> makes one without starting the console, for a service or a script.
        </p>
        <h3 id="windows">On Windows</h3>
        <p>
          Hive runs inside WSL, the Linux that Windows runs. Get WSL once, in PowerShell opened as administrator, and restart when it asks:
        </p>
        <CodeBlock code="wsl --install" lang="powershell" title="In PowerShell" />
        <p>
          Then open the Linux it set up, and run the install line above in it. The console opens in your Windows browser at the same address.{" "}
          <Link href="/docs/how-to/windows/">Run Hive on Windows</Link> has every step, and how to keep Hive running once the window is closed.
        </p>
        <h3 id="homebrew">With Homebrew</h3>
        <p>
          On macOS and Linux, and inside WSL, Homebrew installs the same binary and puts it on your path itself, so only the last two lines above are
          left to run.
          The formula is called <code>hivefleet</code> because Homebrew has another <code>hive</code>.
        </p>
        <CodeBlock code={INSTALL.blocks.brew} title="With Homebrew" copy={INSTALL.brew} copyWhat="the Homebrew command" />
        <p>
          Homebrew owns the file it installed, so a newer release comes with <code>brew upgrade hivefleet</code>, and <code>hive update</code> says so
          and installs nothing. <Link href="/docs/release/#homebrew">Homebrew</Link> has the rest.
        </p>
        <h3 id="from-source">From source</h3>
        <p>
          With Rust 1.95 or newer and Node.js 22 or newer, one command builds the same binary. Node.js is there because the build exports the console and
          embeds it in the binary.
        </p>
        <CodeBlock code={INSTALL.blocks.source} title="From source" copy={INSTALL.source} copyWhat="the command that builds from source" />
        <p>
          Later, with the installer or a build from source, <code>hive update</code> installs a newer release: it checks the download against the release&apos;s checksums, tries the new binary, and
          keeps the old one beside it as <code>hive.prev</code>. Run by hand on a fresh install, it looks at the project&apos;s own releases on GitHub for
          that one run, and says so. To have Hive look once a day by itself, tell it where the releases are:
        </p>
        <CodeBlock code="hive settings set update.latest_url https://api.github.com/repos/lamplitlabs/hive/releases/latest" />
        <p>
          <Link href="/docs/release/">Releases and hive update</Link> has the details.
        </p>

        <H2 id="console">2. Open the console</H2>
        <p>
          <code>hive hub</code> serves the console at <code>http://127.0.0.1:8765</code>. Open that address in your browser. Only this computer can reach
          it. It serves the console and nothing more: the runner, which plans jobs and starts agents, is started separately (step 5).
        </p>
        <p>
          On a new machine, Home shows the card <strong>Set up Hive</strong> with three steps, in this order: <strong>Connect a model provider</strong>,{" "}
          <strong>Add a project</strong> and <strong>Keep Hive running</strong>. Steps 3 to 5 below are those three; this page gives the command for a
          terminal as well.
        </p>
        <p>
          <Link href="/docs/console/">The console</Link> describes every page. On a Mac the same console also comes as an app of its own:{" "}
          <Link href="/docs/mac-app/">the Mac app</Link>.
        </p>

        <H2 id="model">3. Connect a model</H2>
        <p>
          Agents need a model. GitHub Copilot is the default. In the console, <strong>Connect a model provider</strong> on the card opens the same dialog
          as <strong>Settings → Harness → Connect GitHub Copilot</strong>. For GitHub Copilot it starts GitHub&apos;s device login: it shows a code, you
          enter it on github.com, and every agent on this machine runs with that login from then on. From a terminal:
        </p>
        <CodeBlock
          code={[
            "hive harness login                     # GitHub's device flow: open the address, type the code",
            "hive harness status                    # the provider, the login, the models and limits in force",
          ].join("\n")}
        />
        <p>Another provider takes a key instead. Choose it, then hand it the key on standard input:</p>
        <CodeBlock
          code={[
            "hive settings set provider anthropic",
            "hive harness login --provider anthropic --with-token   # reads the key on stdin",
          ].join("\n")}
        />
        <p>
          A key is kept under <code>~/.hive/private/providers/</code>, readable by you alone, and is never echoed.{" "}
          <Link href={`${PAGES.harness.path}#providers`}>Every provider Hive knows</Link> is on the harness page.
        </p>

        <H2 id="project">4. Add a project</H2>
        <p>
          <strong>Add a project</strong> on the card takes the path of a checkout and a test command, such as <code>npm test</code> or{" "}
          <code>cargo test</code>. The test command is optional: when one is set, it must pass before anything is delivered. Left empty, the project
          takes the check its onboarding suggests, and with neither no end check runs. Adding the project queues its onboarding: an agent reads the
          repository, runs that check, posts its first findings and writes down the environment a fresh machine needs. From a terminal, in the checkout:
        </p>
        <CodeBlock code="hive init --name my-app --verify 'npm test'" />

        <H2 id="runner">5. Keep Hive running</H2>
        <p>
          The runner is the part of Hive that plans jobs and starts agents. Until it runs, a task waits in the queue. <strong>Keep Hive running</strong>,
          the third step of the card, installs the runner and the console as user services (launchd on macOS, <code>systemd --user</code> on Linux), so
          neither ends with your terminal. From a terminal:
        </p>
        <CodeBlock
          code={[
            "hive service install     # the runner and the console as user services",
            "hive daemon              # or: the runner alone, in a second terminal, installing nothing",
          ].join("\n")}
        />
        <p>
          The button in the console hands the console over to the service by itself. After <code>hive service install</code> in a terminal, stop the{" "}
          <code>hive hub</code> you started by hand: the service&apos;s console cannot take the address while the first one holds it, and tries again
          every 30 seconds. When the card is gone, Home shows a notice with a button: <strong>Start runner</strong> while no runner runs,{" "}
          <strong>Keep Hive running</strong> while one runs in a terminal only.
        </p>
        <p>
          <Link href="/docs/runner/">The runner</Link> covers the services, self-update, backups and what happens when a run stalls.
        </p>

        <H2 id="task">6. Give the lead a task</H2>
        <p>
          <strong>New task</strong> asks for a title, what you want, and a priority. With the runner running, the project&apos;s lead agent picks the task
          up: it cuts the steps and the agents begin. Open the task and watch every command, every file and every word as it happens, and type into the
          box when you want to steer.
        </p>
        <CodeBlock
          code={["hive task new --project my-app \\", "  --title 'Rate-limit the public API' \\", "  --brief 'Per key, 600 a minute, with the usual headers.'"].join(
            "\n",
          )}
        />

        <H2 id="decide">7. Say yes or no</H2>
        <p>
          What needs your yes or no waits on the <strong>For you</strong> page, and Home shows the first five under <strong>Needs you</strong>: a
          question, a proposal, a request for more machines, a hard stop. A yes-or-no question shows two buttons and an open one an answer box, with the agents&apos;
          reasoning beside them. Each waits for your answer, and everything else carries on. There is one exception: when the self-improvement loop pauses
          because its fixtures tell nothing apart and a day passes with no answer, it answers that one question itself (
          <Link href={`${PAGES.rsi.path}#stops`}>Hard stops and the kill switch</Link>).
        </p>
        <p>
          The work reaches GitHub as pull requests. A review agent merges the ones the agents may decide on, and the console shows you the rest under{" "}
          <strong>Needs your review</strong>, on the task and on the Sessions page. Set <code>review.merge</code> to <code>ask</code> in Settings to see every one.{" "}
          <Link href="/docs/landing/">Delivery</Link> and <Link href="/docs/review/">the reviewer</Link> say how.
        </p>

        <H2 id="next">Where to go next</H2>
        <ul>
          {firstProject && (
            <li>
              <Link href={firstProject.path}>{firstProject.title}</Link>: steps 2 to 5 of this page again, slower, with what you see at each step and what
              to do when one does not work.
            </li>
          )}
          {daily.length > 0 && (
            <li>
              The guides for every day:{" "}
              {daily.map((d, i) => (
                <span key={d.path}>
                  {i > 0 && (i === daily.length - 1 ? " and " : ", ")}
                  <Link href={d.path}>{d.title}</Link>
                </span>
              ))}
              .{" "}
              {guides && (
                <>
                  <Link href={guides.path}>{guides.label}</Link> lists the rest.
                </>
              )}
            </li>
          )}
          {trouble && (
            <li>
              <Link href={trouble.path}>{trouble.title}</Link>: where to look when something does not do what you expected.
            </li>
          )}
          <li>
            <Link href={PAGES.harness.path}>The harness</Link>: how an agent runs, and how you limit it.
          </li>
          <li>
            <Link href={PAGES.rsi.path}>Self-improvement</Link>: the loop, and the constitution it runs under.
          </li>
          <li>
            <Link href={PAGES.machines.path}>Machines</Link>: pair a second machine when the work needs one.
          </li>
          <li>
            <Link href={PAGES.docs.path}>The documentation</Link>
            {commands && (
              <>
                , with <Link href={commands.path}>every command</Link>
              </>
            )}
            {settings && (
              <>
                {" "}
                and <Link href={settings.path}>every setting</Link>
              </>
            )}
            .
          </li>
        </ul>
        <p>
          The store is one SQLite file, <code>~/.hive/hive.sqlite</code>. Once a month, rows older than 90 days that nothing reads back move to{" "}
          <code>hive-archive.sqlite</code> beside it. Set <code>HIVE_HOME</code> to keep both in another folder.
        </p>
      </Article>
    </Frame>
  );
}
