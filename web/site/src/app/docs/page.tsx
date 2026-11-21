import type { Metadata } from "next";
import Link from "@/components/link";
import { ArrowRightIcon } from "lucide-react";
import { Frame } from "@/components/chrome";
import { DocsSearch } from "@/components/docs-search";
import { PageCrumbs, PageHero } from "@/components/page-hero";
import { loadDocs } from "@/lib/docs";
import { pageMetadata } from "@/lib/seo";
import { PAGES, SITE } from "@/lib/site";

export const metadata: Metadata = pageMetadata(PAGES.docs);

const OVERVIEWS = [
  { page: PAGES.install, text: "The install line, building from source, and the first task step by step." },
  { page: PAGES.harness, text: "The tool loop, limits, deny rules, hooks, MCP servers, skills and providers." },
  { page: PAGES.rsi, text: "Suggestions, votes, experiments, private tests and the constitution." },
  { page: PAGES.machines, text: "Pairing over SSH, join codes, and how a job finds its machine." },
];

export default function DocsIndex() {
  const { groups, docs } = loadDocs();
  return (
    <Frame current={PAGES.docs.path}>
      <PageCrumbs page={PAGES.docs} />
      <PageHero
        eyebrow="Documentation"
        title="Hive documentation"
        lead="The reference for every part of Hive, one page per part. These pages are the Markdown files of the repository's docs folder, rendered as they are, so the site and the repository never say different things."
      >
        <DocsSearch className="mt-8 max-w-xl" />
      </PageHero>

      <div className="mx-auto max-w-6xl px-5 py-14 sm:px-8">
        <section aria-labelledby="overviews">
          <h2 id="overviews" className="text-xl font-semibold tracking-tight">
            Start with an overview
          </h2>
          <ul className="mt-5 grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
            {OVERVIEWS.map(({ page, text }) => (
              <li key={page.path}>
                <Link href={page.path} className="group flex h-full flex-col rounded-card border border-border bg-card p-5 shadow-card hover:border-teal">
                  <span className="flex items-center justify-between font-semibold tracking-tight">
                    {page.nav} <ArrowRightIcon className="size-4 text-muted-foreground group-hover:text-link" aria-hidden="true" />
                  </span>
                  <span className="mt-2 text-sm leading-relaxed text-muted-foreground">{text}</span>
                </Link>
              </li>
            ))}
          </ul>
        </section>

        {groups.map((g) => {
          const id = `group-${g.title.toLowerCase().replace(/[^a-z0-9]+/g, "-")}`;
          return (
            <section key={g.title} aria-labelledby={id} className="mt-14">
              <h2 id={id} className="text-xl font-semibold tracking-tight">
                {g.title}
              </h2>
              <ul className="mt-5 grid gap-x-10 gap-y-7 md:grid-cols-2">
                {g.docs.map((d) => (
                  <li key={d.path}>
                    <Link href={d.path} className="font-medium text-link hover:underline">
                      {d.title}
                    </Link>
                    <p className="mt-1.5 text-[15px] leading-relaxed text-muted-foreground">{d.description}</p>
                  </li>
                ))}
              </ul>
            </section>
          );
        })}

        <p className="mt-16 border-t border-border pt-6 text-sm text-muted-foreground">
          {docs.length} pages, rendered from{" "}
          <a href={`${SITE.repo}/tree/main/docs`} className="link" rel="noopener noreferrer">
            the docs folder of the repository
          </a>
          . Each page has a link to edit it on GitHub.
        </p>
      </div>
    </Frame>
  );
}
