import type { ReactNode } from "react";
import { JsonLd } from "@/components/json-ld";
import { breadcrumbJsonLd } from "@/lib/seo";
import type { SitePage } from "@/lib/site";

/** The top band of a page, on the night field: what the page is about, in one heading and one paragraph. */
export function PageHero({ eyebrow, title, lead, children }: { eyebrow: string; title: ReactNode; lead: ReactNode; children?: ReactNode }) {
  return (
    <section className="night night-field border-b border-border">
      <div className="mx-auto max-w-6xl px-5 pt-12 pb-14 sm:px-8 md:pt-16 md:pb-16">
        <p className="font-mono text-xs font-medium tracking-[0.18em] text-teal uppercase">{eyebrow}</p>
        <h1 className="mt-4 max-w-3xl text-4xl font-semibold tracking-tight text-balance sm:text-5xl">{title}</h1>
        <p className="mt-5 max-w-2xl text-lg leading-relaxed text-pretty text-muted-foreground">{lead}</p>
        {children}
      </div>
    </section>
  );
}

/** Where a page one step under the front page sits, for search engines. */
export function PageCrumbs({ page }: { page: SitePage }) {
  return (
    <JsonLd
      data={breadcrumbJsonLd([
        { name: "Hive", path: "/" },
        { name: page.nav, path: page.path },
      ])}
    />
  );
}

export type Section = { id: string; title: string };

/**
 * A page that explains: its sections listed down the side on a wide screen,
 * the text itself in the same measure and type as the docs.
 */
export function Article({ sections, children }: { sections: readonly Section[]; children: ReactNode }) {
  return (
    <div className="mx-auto grid max-w-6xl gap-10 px-5 py-12 sm:px-8 lg:grid-cols-[12rem_minmax(0,1fr)] lg:gap-12 lg:py-16">
      <nav aria-label="On this page" className="hidden lg:block">
        <div className="sticky top-20">
          <p className="text-xs font-semibold tracking-[0.14em] text-muted-foreground uppercase">On this page</p>
          <ul className="mt-3 space-y-1 border-l border-border text-sm">
            {sections.map((s) => (
              <li key={s.id}>
                <a href={`#${s.id}`} className="-ml-px block border-l border-transparent py-1 pl-4 text-muted-foreground hover:border-teal hover:text-foreground">
                  {s.title}
                </a>
              </li>
            ))}
          </ul>
        </div>
      </nav>
      <div className="prose min-w-0">{children}</div>
    </div>
  );
}

/** A section heading with its anchor, as the docs pages have them. */
export function H2({ id, children }: { id: string; children: string }) {
  return (
    <h2 id={id}>
      {children}
      <a className="anchor" href={`#${id}`} aria-hidden="true" tabIndex={-1} />
    </h2>
  );
}
