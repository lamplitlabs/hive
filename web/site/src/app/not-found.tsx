import type { Metadata } from "next";
import Link from "@/components/link";
import { Frame } from "@/components/chrome";
import { HiveMark } from "@/components/hive-mark";
import { NOT_FOUND, PAGES } from "@/lib/site";

export const metadata: Metadata = {
  title: { absolute: NOT_FOUND.title },
  description: NOT_FOUND.description,
  robots: { index: false, follow: false },
};

export default function NotFound() {
  return (
    <Frame>
      <section className="night night-field flex min-h-[70dvh] items-center">
        <div className="mx-auto flex max-w-6xl flex-col items-center px-5 py-24 text-center sm:px-8">
          <HiveMark className="mark-sweep mark-glow size-16" />
          <p className="mt-8 font-mono text-xs font-medium tracking-[0.18em] text-teal uppercase">404</p>
          <h1 className="mt-3 text-3xl font-semibold tracking-tight sm:text-4xl">There is no page here.</h1>
          <p className="mt-4 max-w-md text-pretty text-muted-foreground">
            The address may be old or mistyped. The front page says what Hive is, and the documentation index lists every page of the docs.
          </p>
          <div className="mt-8 flex flex-wrap justify-center gap-3">
            <Link href="/" className="btn btn-primary">
              Front page
            </Link>
            <Link href={PAGES.docs.path} className="btn btn-quiet">
              Documentation
            </Link>
            <Link href={PAGES.install.path} className="btn btn-quiet">
              Install
            </Link>
          </div>
        </div>
      </section>
    </Frame>
  );
}
