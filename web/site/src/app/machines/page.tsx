import type { Metadata } from "next";
import Link from "@/components/link";
import { Frame } from "@/components/chrome";
import { CodeBlock } from "@/components/code";
import { GUIDES_SECTION, GuideLinks } from "@/components/guide-links";
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
  GUIDES_SECTION,
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
            "hive executor label gpu-box +cuda              # add a word on a paired machine",
            "hive executor label gpu-box -cuda              # remove a word",
            "hive executor label gpu-box +cuda,-gpu         # add one word and remove another",
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
            "hive settings set machine.reserve on   # this machine takes only jobs that need what it offers",
            "hive executor reserve gpu-box          # the same for a paired machine; --off releases it",
          ].join("\n")}
        />
        <p>
          A job whose needs no machine offers waits in the queue, and Hive asks you once: pair or label a machine that offers them, or change the needs.
        </p>

        <H2 id="pair">Pair a Linux machine over SSH: hive pair</H2>
        <p>
          <code>hive pair</code> sets up a Linux machine as one of yours, over the SSH login of an admin on that machine. The name is required, and the
          admin needs passwordless sudo. It refuses anything but Linux: a Mac, or any other machine, joins with <code>hive-hand</code>, in the next
          section.
        </p>
        <CodeBlock code="hive pair ADMIN@HOST --name NAME [--workdir /srv/hive] [--workers N] [--projects a,b|*]" />
        <p>
          It reads the machine first and refuses, before it changes anything, when the machine is not fit: not Linux, no passwordless sudo, a work folder
          that already holds clones.
        </p>
        <p>
          It copies a Linux <code>hive</code> to the machine. When Hive itself runs on Linux with the same processor, it sends its own program. Otherwise
          it needs one from you: set <code>HIVE_HANDS_BINARY</code> to a Linux <code>hive</code> for that machine&apos;s processor (the{" "}
          <code>hive</code> inside the release&apos;s Linux archive), or put that file beside your <code>hive</code> as <code>hive-linux-amd64</code> or{" "}
          <code>hive-linux-arm64</code>. From a checkout of Hive&apos;s source it can cross-build one. With none of these it stops and says so.
        </p>
        <p>Then it changes these things, and every step reports <code>kept</code> or <code>done</code>, so pairing again is safe:</p>
        <ul>
          <li>
            On the machine: a <code>hive</code> user; the program as <code>/usr/local/bin/hive-hands</code>, with <code>/usr/local/bin/hive</code> linked
            to it; a git config and <code>mise</code> in that user&apos;s home; a work directory; one line in that user&apos;s{" "}
            <code>authorized_keys</code> that lets Hive&apos;s key run the hands commands and nothing else; and an sshd drop-in,{" "}
            <code>/etc/ssh/sshd_config.d/60-hive-hands.conf</code>, after which sshd is reloaded.
          </li>
          <li>
            On your machine: a <code>Host</code> block in <code>~/.ssh/config</code>, and the machine&apos;s host keys in{" "}
            <code>~/.ssh/known_hosts</code>.
          </li>
        </ul>
        <p>
          A newly paired machine has no model login and takes no work until it has one. Log it in, then probe it: a machine that passes the probe takes
          work. <code>--workers</code> is only the starting count: each probe reads the machine&apos;s cores and memory and sets its slots to the smaller
          of the cores and one run per 3 GB.
        </p>
        <CodeBlock
          code={[
            "hive pair ADMIN@HOST --name NAME --inspect    # read the machine and print the scripts; apply nothing",
            "hive executor login NAME --admin ADMIN@HOST   # a model login for the machine; --with-token reads stdin",
            "hive executor probe NAME                      # check a machine again",
            "hive remove executor NAME                     # the undo",
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
          and one join, with the line to run on the machine. Here it is with an address the machine can reach in place of the one the console shows:
        </p>
        <CodeBlock
          code={[
            "hive-hand join --brain https://hive.example.com amber-river-cobalt-maple-orbit-lantern",
            "hive-hand dial          # every later start: keep this running (a launchd or systemd service)",
          ].join("\n")}
        />
        <p>
          <code>join</code> makes a device key on the machine, sends only its public half, and then dials in itself: it stays connected and does not
          return. <code>dial</code> is what you keep running on every later start. It signs a one-time challenge with that key and then waits for work;
          while the console cannot be reached it tries again, waiting longer each time. The Machines page shows the machine&apos;s last call and a{" "}
          <strong>Revoke</strong> button: a revoked machine is refused, and coming back takes a new code.
        </p>
        <p>
          <code>hive-hand</code> enrols the machine and carries the jobs. An agent itself runs on the harness, which is in <code>hive</code>, so a machine
          that should take work also needs <code>hive</code> on its PATH and a model login of its own. The installer puts both programs in place: once as
          it is, once with <code>--hand</code>.
        </p>
        <p>
          <code>--brain</code> is the address the machine reaches your Hive on. The line the console shows carries the address your browser used, so it
          usually reads <code>http://127.0.0.1:8765</code>: change it to an address the other machine can reach. The console listens on{" "}
          <code>127.0.0.1</code> only, so another machine reaches it only through something you set up on the machine Hive runs on: an SSH tunnel, or a
          proxy with TLS there. A private network alone does not reach a port bound to <code>127.0.0.1</code>. A tunnel reaches the whole console,
          not only the routes <code>hive-hand</code> uses, so keep it between your own machines. Under a name of your own, through a proxy, the
          console answers the routes <code>hive-hand</code> uses and nothing else, until you list that name in the setting <code>hub.hosts</code>:
          it refuses every other host name, so that a web page cannot reach it by pointing a name at <code>127.0.0.1</code>.
        </p>
        <p>
          An SSH tunnel is the short way. On the machine that joins, forward a port of its own to the console and name that port. Keep the tunnel up for
          as long as the machine should take work:
        </p>
        <CodeBlock
          code={[
            "ssh -N -L 8765:127.0.0.1:8765 YOU@HOST &                # HOST is the machine Hive runs on",
            "hive-hand join --brain http://127.0.0.1:8765 CODE       # port 8765 here now reaches the console",
          ].join("\n")}
        />

        <H2 id="quiet">When a machine goes quiet</H2>
        <p>
          A machine that stops answering gets no new work. Hive probes it again after a minute, waits longer after each probe that fails, up to thirty
          minutes, and puts it back in rotation with the first probe that passes. A job whose machine fell silent in the middle of a run goes back to the
          queue.
        </p>

        <GuideLinks names={["add-a-machine"]} />

        <H2 id="reference">The full reference</H2>
        <ul>
          <li>
            <Link href="/docs/machines/">Machines</Link>: needs and offers, pairing, the join protocol, heartbeats, moving a project, removing projects
            and machines.
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
