import Link from "@/components/link";
import type { ReactNode } from "react";
import { DocsSearch } from "@/components/docs-search";
import { loadDocs } from "@/lib/docs";
import type { TocEntry } from "@/lib/markdown";

/** The docs in their groups, the page you are on marked. */
function Pages({ current }: { current: string }) {
  const { groups } = loadDocs();
  return (
    <>
      {groups.map((g) => (
        <div key={g.title}>
          <p className="text-xs font-semibold tracking-[0.14em] text-muted-foreground uppercase">{g.title}</p>
          <ul className="mt-2 space-y-px border-l border-border" aria-label={g.title}>
            {g.docs.map((d) => (
              <li key={d.path}>
                <Link
                  href={d.path}
                  aria-current={current === d.path ? "page" : undefined}
                  className="-ml-px block border-l border-transparent py-1.5 pl-4 text-sm leading-snug text-muted-foreground hover:border-teal hover:text-foreground aria-[current=page]:border-teal aria-[current=page]:font-medium aria-[current=page]:text-foreground"
                >
                  {d.label}
                </Link>
              </li>
            ))}
          </ul>
        </div>
      ))}
    </>
  );
}

/** The h2 and h3 headings of the page. */
export function Toc({ toc }: { toc: readonly TocEntry[] }) {
  return (
    <ul className="space-y-px border-l border-border text-sm">
      {toc.map((e) => (
        <li key={e.id}>
          <a
            href={`#${e.id}`}
            className={`-ml-px block border-l border-transparent py-1 leading-snug text-muted-foreground hover:border-teal hover:text-foreground ${e.level === 3 ? "pl-7 text-[13px]" : "pl-4"}`}
          >
            {e.text}
          </a>
        </li>
      ))}
    </ul>
  );
}

/**
 * A page of the docs: the search and every page down the left (folded behind
 * one line on a small screen), the text in the middle, the page's own
 * headings down the right when the screen has room.
 */
export function DocsShell({ current, toc, children }: { current: string; toc?: readonly TocEntry[]; children: ReactNode }) {
  return (
    <div className="mx-auto w-full max-w-[92rem] px-5 sm:px-8 lg:grid lg:grid-cols-[15rem_minmax(0,1fr)] lg:gap-12 xl:grid-cols-[15rem_minmax(0,1fr)_14rem]">
      <aside aria-label="Documentation" className="pt-6 lg:sticky lg:top-14 lg:h-[calc(100dvh-3.5rem)] lg:overflow-y-auto lg:pt-9 lg:pr-1 lg:pb-10">
        <DocsSearch />
        <nav aria-label="Documentation pages" className="mt-7 hidden space-y-7 lg:block">
          <Pages current={current} />
        </nav>
        <details className="mt-3 rounded-xl border border-border bg-card lg:hidden">
          <summary className="px-4 py-2.5 text-sm font-medium">All documentation pages</summary>
          <nav aria-label="Documentation pages, small screens" className="space-y-6 border-t border-border px-4 py-4">
            <Pages current={current} />
          </nav>
        </details>
      </aside>
      <div className="min-w-0 pt-8 pb-16 lg:pt-10">{children}</div>
      {toc && toc.length > 1 && (
        <aside aria-label="On this page" className="hidden xl:block">
          <nav aria-label="Headings of this page" className="sticky top-14 max-h-[calc(100dvh-3.5rem)] overflow-y-auto pt-10 pb-10">
            <p className="mb-3 text-xs font-semibold tracking-[0.14em] text-muted-foreground uppercase">On this page</p>
            <Toc toc={toc} />
          </nav>
        </aside>
      )}
    </div>
  );
}
