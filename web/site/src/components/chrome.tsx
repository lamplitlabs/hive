import Link from "@/components/link";
import type { ReactNode } from "react";
import { ArrowUpRightIcon, MenuIcon } from "lucide-react";
import { HiveTile } from "@/components/hive-mark";
import { JsonLd } from "@/components/json-ld";
import { GUIDES_DIR, optionalDoc } from "@/lib/docs";
import { siteJsonLd } from "@/lib/seo";
import { PAGES, repoFile, SITE } from "@/lib/site";

type NavItem = { path: string; nav: string };

/**
 * The header's links. The guides are pages of the docs with a place of their
 * own up here, because they are what a newcomer opens after the install page.
 */
function navItems(): NavItem[] {
  const guides = optionalDoc(GUIDES_DIR);
  return [PAGES.install, ...(guides ? [{ path: guides.path, nav: "Guides" }] : []), PAGES.harness, PAGES.rsi, PAGES.machines, PAGES.docs];
}

/** The same links without the install page: on a wide screen that one is the header's button, the one thing the site asks a visitor to do. */
function barItems(): NavItem[] {
  return navItems().filter((p) => p.path !== PAGES.install.path);
}

/** The docs the footer names, in this order; one that is not there (yet) is left out. */
const FOOTER_DOCS = ["concepts", "faq", "console", "commands", "settings", "board", "landing", "runner", "security", "review"];

/** The guides the footer names, by the file's name under docs/how-to/. */
const FOOTER_GUIDES = ["first-project", "tasks", "pull-requests", "self-improvement-loop", "add-a-machine", "models", "troubleshooting"];

/**
 * Every page: the skip link, the header with this page marked in it, the
 * content as the page's one <main>, the footer. A page passes the path it is
 * (or the section it belongs to), so the header is plain HTML with
 * aria-current where it belongs and needs no script to know where it is.
 *
 * The id of <main> has a dot in it because no heading of a doc can: GitHub's
 * heading ids drop dots, so the skip link's target never collides with an
 * anchor of the page.
 */
export function Frame({ current, children }: { current?: string; children: ReactNode }) {
  return (
    <>
      <a href="#site.main" className="skip-link">
        Skip to the content
      </a>
      <SweepDefs />
      <Header current={current} />
      <main id="site.main" tabIndex={-1} className="flex-1 outline-none">
        {children}
      </main>
      <Footer />
      <JsonLd data={siteJsonLd()} />
    </>
  );
}

/** The brand sweep as a paint the mark can be filled with (`.mark-sweep`): mint, sky, violet, top left to bottom right. */
function SweepDefs() {
  return (
    <svg width="0" height="0" className="absolute" aria-hidden="true" focusable="false">
      <defs>
        <linearGradient id="hive-sweep" gradientUnits="userSpaceOnUse" x1="24" y1="24" x2="232" y2="232">
          <stop offset="0" stopColor="#50f4c5" />
          <stop offset="0.52" stopColor="#11bff0" />
          <stop offset="1" stopColor="#877af7" />
        </linearGradient>
      </defs>
    </svg>
  );
}

function Header({ current }: { current?: string }) {
  const NAV = navItems();
  const BAR = barItems();
  const link = "rounded-full px-3 py-1.5 text-sm font-medium text-muted-foreground transition-colors hover:bg-muted hover:text-foreground aria-[current=page]:bg-muted aria-[current=page]:text-foreground";
  return (
    <header className="night sticky top-0 z-30 border-b border-border bg-[var(--night-1)]">
      <div className="mx-auto flex h-14 max-w-6xl items-center gap-2 px-5 sm:px-8">
        <Link href="/" className="mr-3 flex min-h-9 items-center gap-2.5 rounded-md font-semibold tracking-tight" aria-current={current === "/" ? "page" : undefined}>
          <HiveTile className="size-7" />
          <span className="text-[17px]">Hive</span>
        </Link>
        <nav aria-label="Main" className="hidden items-center gap-0.5 md:flex">
          {BAR.map((p) => (
            <Link key={p.path} href={p.path} className={link} aria-current={current === p.path ? "page" : undefined}>
              {p.nav}
            </Link>
          ))}
        </nav>
        <div className="ml-auto flex items-center gap-1.5">
          <a href={SITE.repo} className={`${link} hidden items-center gap-1 sm:inline-flex`} rel="noopener noreferrer">
            GitHub <ArrowUpRightIcon className="size-3.5" aria-hidden="true" />
          </a>
          <Link href={PAGES.install.path} className="btn btn-primary btn-sm" aria-current={current === PAGES.install.path ? "page" : undefined}>
            Install
          </Link>
          {/* Small screens: the same links behind one button. Plain anchors, so a tap loads the page and the menu is closed again. */}
          <details className="group relative md:hidden">
            <summary className="flex size-9 list-none items-center justify-center rounded-full text-foreground hover:bg-muted [&::-webkit-details-marker]:hidden" aria-label="Menu">
              <MenuIcon className="size-5" aria-hidden="true" />
            </summary>
            <nav
              aria-label="Main, small screens"
              className="absolute top-11 right-0 z-40 flex w-56 flex-col gap-0.5 rounded-2xl border border-border bg-[var(--night-1)] p-2 shadow-pop"
            >
              {NAV.map((p) => (
                <a key={p.path} href={p.path} className={`${link} rounded-xl`} aria-current={current === p.path ? "page" : undefined}>
                  {p.nav}
                </a>
              ))}
              <a href={SITE.repo} className={`${link} inline-flex items-center gap-1 rounded-xl`} rel="noopener noreferrer">
                GitHub <ArrowUpRightIcon className="size-3.5" aria-hidden="true" />
              </a>
            </nav>
          </details>
        </div>
      </div>
    </header>
  );
}

function Footer() {
  const docs = FOOTER_DOCS.map((slug) => optionalDoc(slug)).filter((d) => d !== undefined).slice(0, 7);
  const guides = FOOTER_GUIDES.map((name) => optionalDoc(`${GUIDES_DIR}/${name}`)).filter((d) => d !== undefined);
  const allGuides = optionalDoc(GUIDES_DIR);
  const pages = [PAGES.install, PAGES.harness, PAGES.rsi, PAGES.machines, PAGES.docs];
  const item = "text-muted-foreground transition-colors hover:text-foreground";
  const head = "text-[13px] font-semibold tracking-[0.12em] text-foreground uppercase";
  return (
    <footer className="night night-field border-t border-border">
      <div className={`mx-auto grid max-w-6xl gap-10 px-5 py-14 text-sm sm:grid-cols-2 sm:px-8 ${guides.length > 0 ? "lg:grid-cols-[minmax(0,1fr)_repeat(4,max-content)]" : "lg:grid-cols-[minmax(0,1fr)_repeat(3,max-content)]"}`}>
        <div>
          <div className="flex items-center gap-2.5">
            <HiveTile className="size-8" />
            <span className="text-lg font-semibold tracking-tight">Hive</span>
          </div>
          <p className="mt-4 max-w-xs leading-relaxed text-muted-foreground text-pretty">
            {SITE.tagline}. Open source, one binary, your own machines.
          </p>
          <p className="mt-4 text-muted-foreground">
            <a href={repoFile("LICENSE")} className="link" rel="noopener noreferrer">
              MIT licence
            </a>
            . Copyright {SITE.year}{" "}
            <a href={SITE.holderUrl} className="link whitespace-nowrap" rel="noopener noreferrer">
              {SITE.holder}
            </a>
            .
          </p>
        </div>
        <nav aria-label="Footer, pages">
          <h2 className={head}>Pages</h2>
          <ul className="mt-4 space-y-2.5">
            <li>
              <Link href="/" className={item}>
                Overview
              </Link>
            </li>
            {pages.map((p) => (
              <li key={p.path}>
                <Link href={p.path} className={item}>
                  {p.nav}
                </Link>
              </li>
            ))}
          </ul>
        </nav>
        {guides.length > 0 && (
          <nav aria-label="Footer, guides">
            <h2 className={head}>Guides</h2>
            <ul className="mt-4 space-y-2.5">
              {guides.map((d) => (
                <li key={d.path}>
                  <Link href={d.path} className={item}>
                    {d.label}
                  </Link>
                </li>
              ))}
              {allGuides && (
                <li>
                  <Link href={allGuides.path} className={item}>
                    {allGuides.label}
                  </Link>
                </li>
              )}
            </ul>
          </nav>
        )}
        <nav aria-label="Footer, documentation">
          <h2 className={head}>Docs</h2>
          <ul className="mt-4 space-y-2.5">
            {docs.map((d) => (
              <li key={d.path}>
                <Link href={d.path} className={item}>
                  {d.label}
                </Link>
              </li>
            ))}
          </ul>
        </nav>
        <nav aria-label="Footer, project">
          <h2 className={head}>Project</h2>
          <ul className="mt-4 space-y-2.5">
            <li>
              <a href={SITE.repo} className={item} rel="noopener noreferrer">
                GitHub repository
              </a>
            </li>
            <li>
              <a href={`${SITE.repo}/releases`} className={item} rel="noopener noreferrer">
                Releases
              </a>
            </li>
            <li>
              <a href={repoFile("CONTRIBUTING.md")} className={item} rel="noopener noreferrer">
                Contributing
              </a>
            </li>
            <li>
              <a href={repoFile("SECURITY.md")} className={item} rel="noopener noreferrer">
                Reporting a vulnerability
              </a>
            </li>
            <li>
              <a href={repoFile("LICENSE")} className={item} rel="noopener noreferrer">
                Licence (MIT)
              </a>
            </li>
            <li>
              <a href="/llms.txt" className={item}>
                llms.txt
              </a>
            </li>
          </ul>
        </nav>
      </div>
    </footer>
  );
}
