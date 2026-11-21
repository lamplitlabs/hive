import type { Metadata } from "next";
import Link from "@/components/link";
import { Frame } from "@/components/chrome";
import { CodeBlock, CopyLine } from "@/components/code";
import { Article, H2, PageHero } from "@/components/page-hero";
import { optionalDoc } from "@/lib/docs";
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
  return (
    <Frame current={PAGES.install.path}>
      <PageHero
        eyebrow="Install"
        title="Install Hive and give it a first task."
        lead="One command installs the binary. The console opens in your browser, on your own machine. A few minutes later a project's lead agent is at work on your first task."
      >
        <div className="mt-8 max-w-xl">
          <CopyLine command={INSTALL.script} what="the install command" />
        </div>
      </PageHero>

      <Article sections={SECTIONS}>
        <H2 id="need">What you need</H2>
        <ul>
          <li>
            macOS or Linux. Windows is not supported today; <Link href="/docs/windows-port/">a port is in progress</Link>.
          </li>
          <li>To run agents: a GitHub Copilot seat, or an API key for another model provider.</li>
          <li>Nothing else. There is no other agent tool to install or keep logged in.</li>
        </ul>

        <H2 id="install">1. Install</H2>
        <CodeBlock code={INSTALL.blocks.script} title="macOS and Linux" copy={INSTALL.script} />
        <p>
          The installer is one shell script. It is{" "}
          <a href={repoFile("scripts/install.sh")} rel="noopener noreferrer">
            <code>scripts/install.sh</code>
          </a>{" "}
          in the repository, and this site serves the same file, so you can read it before you run it. Then put the binary on your path and look at it:
        </p>
        <CodeBlock code={INSTALL.blocks.after} title="Then" />
        <h3 id="from-source">From source</h3>
        <p>
          With Rust 1.95 or newer and Node.js 22 or newer, one command builds the same binary. Node.js is there because the build exports the console and
          embeds it in the binary.
        </p>
        <CodeBlock code={INSTALL.blocks.source} title="From source" copy={INSTALL.source} />
        <p>
          Later, <code>hive update</code> installs a newer release: it checks the download against the release&apos;s checksums, tries the new binary, and
          keeps the old one beside it. <Link href="/docs/release/">Releases and hive update</Link> has the details.
        </p>

        <H2 id="console">2. Open the console</H2>
        <p>
          <code>hive hub</code> serves the console at <code>http://127.0.0.1:8765</code>. Open that address in your browser. Only this computer can reach
          it. Each step below has a button there; this page gives the command for a terminal as well.
        </p>
        <p>
          <Link href="/docs/console/">The console</Link> describes every page. On a Mac the same console also comes as an app of its own:{" "}
          <Link href="/docs/mac-app/">the Mac app</Link>.
        </p>

        <H2 id="model">3. Connect a model</H2>
        <p>
          Agents need a model. GitHub Copilot is the default. In the console, <strong>Settings → Harness</strong> starts GitHub&apos;s device login: it
          shows a code, you enter it on github.com, and every agent on this machine runs with that login from then on. From a terminal:
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
          <strong>Add project</strong> on Home takes the path of a checkout and the test command that must pass before anything is delivered, such as{" "}
          <code>npm test</code> or <code>cargo test</code>. It then queues the onboarding: an agent reads the repository, runs that check, posts its first
          findings and writes down the environment a fresh machine needs. From a terminal, in the checkout:
        </p>
        <CodeBlock code="hive init --name my-app --verify 'npm test'" />

        <H2 id="runner">5. Keep Hive running</H2>
        <p>
          The runner is the part of Hive that plans jobs and starts agents. <strong>Keep Hive running</strong>, under <strong>Details for this machine</strong>{" "}
          on Home, installs the runner and the console as user services (launchd on macOS, <code>systemd --user</code> on Linux), so neither ends with your
          terminal. From a terminal:
        </p>
        <CodeBlock code="hive service install" />
        <p>
          <Link href="/docs/runner/">The runner</Link> covers the services, self-update, backups and what happens when a run stalls.
        </p>

        <H2 id="task">6. Give the lead a task</H2>
        <p>
          <strong>New task</strong> asks for a title, what you want, and a priority. The project&apos;s lead agent starts on it at once: it cuts the steps and
          the agents begin. Open the task and watch every command, every file and every word as it happens, and type into the box when you want to steer.
        </p>
        <CodeBlock
          code={["hive task new --project my-app \\", "  --title 'Rate-limit the public API' \\", "  --brief 'Per key, 600 a minute, with the usual headers.'"].join(
            "\n",
          )}
        />

        <H2 id="decide">7. Say yes or no</H2>
        <p>
          Anything that needs you waits on Home under <strong>For you</strong>, with the agents&apos; reasoning beside the buttons: a question, a proposal, a
          request for more machines, a hard stop. Nothing there moves until you answer. Everything else carries on.
        </p>
        <p>
          The work reaches GitHub as pull requests. A review agent merges the ones the agents may decide on, and the console shows you the rest under{" "}
          <strong>Needs your review</strong>. Set <code>review.merge</code> to <code>ask</code> in Settings to see every one.{" "}
          <Link href="/docs/landing/">Delivery</Link> and <Link href="/docs/review/">the reviewer</Link> say how.
        </p>

        <H2 id="next">Where to go next</H2>
        <ul>
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
          The store is one SQLite file, <code>~/.hive/hive.sqlite</code>. <code>HIVE_HOME</code> names another folder than <code>~/.hive</code> when you
          want one.
        </p>
      </Article>
    </Frame>
  );
}
