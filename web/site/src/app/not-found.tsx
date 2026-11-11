import Link from "next/link";
import { HiveTile } from "@/components/hive-mark";

export default function NotFound() {
  return (
    <section className="surface-hero flex flex-1 items-center">
      <div className="mx-auto flex max-w-6xl flex-col items-center px-5 py-24 text-center sm:px-8">
        <HiveTile className="size-12" />
        <h1 className="mt-5 text-3xl font-semibold tracking-tight">There is no page here.</h1>
        <p className="mt-3 max-w-md text-pretty text-muted-foreground">
          The address may be old or mistyped. The front page says what Hive is; the sign-in opens your team.
        </p>
        <div className="mt-8 flex gap-3">
          <Link
            href="/"
            className="inline-flex h-10 items-center rounded-full bg-primary px-5 text-sm font-medium text-primary-foreground hover:bg-primary-strong"
          >
            Front page
          </Link>
          <Link
            href="/sign-in/"
            className="inline-flex h-10 items-center rounded-full border border-border bg-card px-5 text-sm font-medium hover:bg-muted"
          >
            Sign in
          </Link>
        </div>
      </div>
    </section>
  );
}
