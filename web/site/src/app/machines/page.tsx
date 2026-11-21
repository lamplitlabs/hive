import type { Metadata } from "next";
import Link from "@/components/link";
import { Frame } from "@/components/chrome";
import { CodeBlock } from "@/components/code";
import { Article, H2, PageCrumbs, PageHero } from "@/components/page-hero";
import { pageMetadata } from "@/lib/seo";
import { PAGES } from "@/lib/site";

export const metadata: Metadata = pageMetadata(PAGES.machines);

const SECTIONS = [
  { id: "needs", title: "Needs and offers" },
  { id: "where", title: "Where a job goes" },
  { id: "pair", title: "Pair over SSH: hive pair" },
  { id: "join", title: "Join codes and hive-hand" },
  { id: "quiet", title: "When a machine goes quiet" },
  { id: "reference", title: "The full reference" },
];

export default function MachinesPage() {
  return (
    <Frame current={PAGES.machines.path}>
      <PageCrumbs page={PAGES.machines} />
      <PageHero
        eyebrow="Machines"
        title="Pair more machines."
        lead="Hive starts on one machine. When the work needs more hands, or something that machine does not have, pair another. A project says what its jobs need, and the queue sends each job to a machine that offers it."
      />

      <Article sections={SECTIONS}>
        <H2 id="needs">What a job needs, what a machine offers</H2>
        <p>
          Machines are not alike. An iOS app needs a Mac with Xcode, a model evaluation wants a GPU, and most jobs want nothing in particular. So a job
          says what it needs, a machine says what it offers, and both use the same plain lowercase words, with no fixed list: <code>macos</code>,{" "}
          <code>linux</code>, <code>arm64</code>, <code>amd64</code>, <code>xcode</code>, <code>docker</code>, <code>gpu</code>, <code>android</code>, the
          name of a tool.
        </p>
        <p>
          A job fits a machine when every word it needs is among the words the machine offers. A job that needs nothing fits every machine that is not
          reserved.
        </p>
        <p>
          <strong>What a machine offers</strong> is its operating system, its chip and its labels. Hive reads the first two by itself and finds{" "}
          <code>xcode</code> and <code>docker</code> on its own. You add the rest:
        </p>
        <CodeBlock
          code={[
            "hive settings set machine.labels gpu,android   # what this machine offers, beyond what Hive finds",
            "hive executor label gpu-box +cuda              # add a word on a paired machine; -word removes one",
            "hive executor list                             # every machine, its slots and what it offers",
          ].join("\n")}
        />
        <p>
          <strong>What a job needs</strong> comes from its project, and one job can differ:
        </p>
        <CodeBlock
          code={[
            "hive init ~/code/ios-app --needs macos,xcode   # every job of the project needs a Mac with Xcode",
            "hive job needs JOB_ID linux,gpu                # one waiting job that needs something else",
            "hive job needs JOB_ID -                        # this one job may run anywhere after all",
          ].join("\n")}
        />
        <p>
          The same words can live in the repository, on the <code>needs:</code> line of <code>.hive/hive.yaml</code>. In the console, a project&apos;s{" "}
          <strong>Runs on</strong> panel shows the words, which machines fit, and a warning when none does.
        </p>

        <H2 id="where">Where a job goes</H2>
        <p>
          The queue fills free slots in an order that protects the scarce machines. Reserved machines come first, and take only the jobs that need them.
          Then come the machines kept for certain projects, then the machine that offers the least: a plain Linux machine before the one with a GPU,
          before the Mac with Xcode. The scarce machine stays free for the work only it can do.
        </p>
        <p>
          Some jobs never leave the machine Hive runs on, whatever they need: the lead, the review and the reflection work against the store, and the
          store is here.
        </p>
        <CodeBlock
          code={[
            "hive settings set machine.reserve on   # this machine takes only the jobs that need something it offers",
            "hive executor reserve gpu-box          # the same for a paired machine; --off releases it",
          ].join("\n")}
        />
        <p>
          A job whose needs no machine offers waits in the queue, and Hive asks you once: pair or label a machine that offers them, or change the needs.
        </p>

        <H2 id="pair">Pair a Linux machine over SSH: hive pair</H2>
        <p>
          <code>hive pair</code> turns a Linux machine into one of yours in one run, over the SSH login of an admin on that machine.
        </p>
        <CodeBlock code="hive pair ADMIN@HOST --name NAME [--workdir /srv/hive] [--workers N] [--projects a,b|*]" />
        <p>
          It reads the machine first and refuses, before it changes anything, when the machine is not fit: not Linux, no passwordless sudo, a work folder
          that already holds clones. Then it creates a <code>hive</code> user there, installs the small program that runs jobs, and writes one line in that
          user&apos;s <code>authorized_keys</code> that lets Hive&apos;s key run that program and nothing else. Every step reports <code>kept</code> or{" "}
          <code>done</code>, so pairing again is safe.
        </p>
        <p>
          A probe then checks the machine, and a machine that passes takes work. <code>--workers</code> is only the starting count: each probe reads the
          machine&apos;s cores and memory and sets its slots to the smaller of the cores and one run per 3 GB.
        </p>
        <CodeBlock
          code={[
            "hive pair ADMIN@HOST --name NAME --inspect   # read the machine and print the scripts; apply nothing",
            "hive executor probe NAME                     # check a machine again",
            "hive remove executor NAME                    # the undo",
          ].join("\n")}
        />

        <H2 id="join">Enrol any machine: join codes and hive-hand</H2>
        <p>
          <code>hive-hand</code> is a second, small program. It holds only what a machine needs to run the jobs it is handed, and none of what decides
          which jobs exist. A machine with it joins once and then calls out for work. There is no open port on it, no change to its SSH server, and no key
          of yours stored on it.
        </p>
        <p>
          In the console, <strong>Machines → Add a machine → Run one command on it → Get a join code</strong> shows a six-word code, good for ten minutes
          and one join, with the line to run on the machine:
        </p>
        <CodeBlock
          code={[
            "hive-hand join --brain https://hive.example.com amber-river-cobalt-maple-orbit-lantern",
            "hive-hand dial          # keep it running (a launchd or systemd service)",
          ].join("\n")}
        />
        <p>
          <code>join</code> makes a device key on the machine and sends only its public half. <code>dial</code> signs a one-time challenge with that key
          and then waits for work. The Machines page shows the machine&apos;s last call and a <strong>Revoke</strong> button: a revoked machine is refused,
          and coming back takes a new code.
        </p>
        <p>
          <code>--brain</code> is the address the machine reaches your Hive on; the line the console shows carries the address your browser used. The
          console listens on <code>127.0.0.1</code> only, so a machine elsewhere needs a way to that address that you set up. TLS is whatever stands in
          front of it.
        </p>

        <H2 id="quiet">When a machine goes quiet</H2>
        <p>
          A machine that stops answering gets no new work. Hive probes it again after a minute, waits longer after each probe that fails, up to thirty
          minutes, and puts it back in rotation with the first probe that passes. A job whose machine fell silent in the middle of a run goes back to the
          queue.
        </p>

        <H2 id="reference">The full reference</H2>
        <ul>
          <li>
            <Link href="/docs/machines/">Machines</Link>: needs and offers, pairing, the join protocol, heartbeats, moving and removing machines.
          </li>
          <li>
            <Link href="/docs/console/#machines">The Machines page</Link> of the console.
          </li>
          <li>
            <Link href="/docs/runner/">The runner</Link>: how jobs are leased and what keeps the machine healthy.
          </li>
          <li>
            <Link href="/docs/security/">Security notes</Link>: where code runs and where data lives.
          </li>
        </ul>
      </Article>
    </Frame>
  );
}
