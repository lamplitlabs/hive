import type { Metadata } from "next";
import Link from "@/components/link";
import { Frame } from "@/components/chrome";
import { GUIDES_DIR, optionalDoc } from "@/lib/docs";
import { NOT_FOUND, PAGES } from "@/lib/site";

export const metadata: Metadata = {
  title: { absolute: NOT_FOUND.title },
  description: NOT_FOUND.description,
  robots: { index: false, follow: false },
};

export default function NotFound() {
  const guides = optionalDoc(GUIDES_DIR);
  return (
    <Frame>
      <section className="night night-field flex min-h-[60dvh] items-center">
        <div className="mx-auto flex max-w-6xl flex-col items-center px-5 py-20 text-center sm:px-8">
          <p className="text-brand font-mono text-7xl leading-none font-semibold tracking-tight sm:text-8xl" aria-hidden="true">
            404
          </p>
          <h1 className="mt-6 text-3xl font-semibold tracking-tight sm:text-4xl">There is no page here.</h1>
          <p className="mt-4 max-w-lg text-pretty text-muted-foreground">
            The address may be old or mistyped. The front page says what Hive is, and the documentation index lists every page of the docs.
          </p>
          <div className="mt-8 flex flex-wrap justify-center gap-3">
            <Link href="/" className="btn btn-primary">
              Front page
            </Link>
            <Link href={PAGES.docs.path} className="btn btn-quiet">
              Documentation
            </Link>
            {guides && (
              <Link href={guides.path} className="btn btn-quiet">
                Guides
              </Link>
            )}
            <Link href={PAGES.install.path} className="btn btn-quiet">
              Install
            </Link>
          </div>
        </div>
      </section>
    </Frame>
  );
}
