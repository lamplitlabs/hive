import type { Metadata } from "next";
import Link from "@/components/link";
import { notFound } from "next/navigation";
import { ArrowLeftIcon, ArrowRightIcon, ChevronRightIcon, SquarePenIcon } from "lucide-react";
import { Frame } from "@/components/chrome";
import { DocsShell, Toc } from "@/components/docs-shell";
import { JsonLd } from "@/components/json-ld";
import { loadDocs } from "@/lib/docs";
import { breadcrumbJsonLd, pageMetadata, techArticleJsonLd } from "@/lib/seo";
import { PAGES } from "@/lib/site";

type Params = { params: Promise<{ slug: string[] }> };

// One page per Markdown file of the repository's docs/, known at build time; any other address is the 404 page.
export const dynamicParams = false;

export function generateStaticParams() {
  return loadDocs().docs.map((d) => ({ slug: d.slug.split("/") }));
}

async function find({ params }: Params) {
  const { slug } = await params;
  return loadDocs().bySlug.get(slug.map(decodeURIComponent).join("/"));
}

export async function generateMetadata(props: Params): Promise<Metadata> {
  const doc = await find(props);
  if (!doc) return {};
  return pageMetadata({ path: doc.path, title: doc.metaTitle, description: doc.description, shareTitle: doc.title }, "article");
}

export default async function DocPage(props: Params) {
  const doc = await find(props);
  if (!doc) notFound();
  const { status, date } = doc.frontMatter;

  return (
    <Frame current={PAGES.docs.path}>
      <JsonLd
        data={breadcrumbJsonLd([
          { name: "Hive", path: "/" },
          { name: "Docs", path: PAGES.docs.path },
          { name: doc.label, path: doc.path },
        ])}
      />
      <JsonLd data={techArticleJsonLd(doc)} />
      <DocsShell current={doc.path} toc={doc.toc}>
        <article>
          <nav aria-label="Breadcrumb">
            <ol className="flex flex-wrap items-center gap-x-1.5 gap-y-1 text-sm text-muted-foreground">
              <li>
                <Link href={PAGES.docs.path} className="hover:text-foreground hover:underline">
                  Docs
                </Link>
              </li>
              <li className="flex items-center gap-1.5">
                <ChevronRightIcon className="size-3.5" aria-hidden="true" />
                {doc.group}
              </li>
              <li className="flex items-center gap-1.5 text-foreground" aria-current="page">
                <ChevronRightIcon className="size-3.5 text-muted-foreground" aria-hidden="true" />
                {doc.label}
              </li>
            </ol>
          </nav>

          <h1
            id={doc.titleId || undefined}
            className="mt-5 max-w-[26ch] scroll-mt-24 text-3xl leading-tight font-semibold tracking-tight text-balance sm:text-4xl [&_code]:font-mono [&_code]:text-[0.9em]"
            dangerouslySetInnerHTML={{ __html: doc.titleHtml }}
          />
          {(status || date) && (
            <p className="mt-3 text-sm text-muted-foreground">
              {status && <>Status: {status}</>}
              {status && date && " · "}
              {date}
            </p>
          )}

          {doc.toc.length > 1 && (
            <details className="mt-6 rounded-xl border border-border bg-card xl:hidden">
              <summary className="px-4 py-2.5 text-sm font-medium">On this page</summary>
              <nav aria-label="Headings of this page, small screens" className="border-t border-border px-4 py-3">
                <Toc toc={doc.toc} />
              </nav>
            </details>
          )}

          <div className="prose mt-8" dangerouslySetInnerHTML={{ __html: doc.html }} />

          <footer className="mt-14 border-t border-border pt-6">
            <a href={doc.editUrl} className="inline-flex items-center gap-2 text-sm font-medium text-link hover:underline" rel="noopener noreferrer">
              <SquarePenIcon className="size-4" aria-hidden="true" /> Edit this page on GitHub
            </a>
            <nav aria-label="Previous and next page" className="mt-6 grid gap-3 sm:grid-cols-2">
              {doc.prev ? (
                <Link href={doc.prev.path} className="group rounded-card border border-border bg-card p-4 hover:border-teal" rel="prev">
                  <span className="flex items-center gap-1.5 text-xs text-muted-foreground">
                    <ArrowLeftIcon className="size-3.5" aria-hidden="true" /> Previous
                  </span>
                  <span className="mt-1 block font-medium group-hover:text-link">{doc.prev.label}</span>
                </Link>
              ) : (
                <span className="hidden sm:block" />
              )}
              {doc.next && (
                <Link href={doc.next.path} className="group rounded-card border border-border bg-card p-4 text-right hover:border-teal" rel="next">
                  <span className="flex items-center justify-end gap-1.5 text-xs text-muted-foreground">
                    Next <ArrowRightIcon className="size-3.5" aria-hidden="true" />
                  </span>
                  <span className="mt-1 block font-medium group-hover:text-link">{doc.next.label}</span>
                </Link>
              )}
            </nav>
          </footer>
        </article>
      </DocsShell>
    </Frame>
  );
}
