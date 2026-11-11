import Link from "next/link";
import { HiveTile } from "@/components/hive-mark";
import { SignInControl } from "@/components/sign-in-control";

const NAV = [
  { href: "/#what", label: "What it does" },
  { href: "/#how", label: "How it works" },
  { href: "/#security", label: "Yours" },
];

export function Header() {
  return (
    <header className="sticky top-0 z-20 border-b border-border/70 bg-background/80 backdrop-blur supports-[backdrop-filter]:bg-background/60">
      <div className="mx-auto flex h-16 max-w-6xl items-center justify-between gap-4 px-5 sm:px-8">
        <Link href="/" className="flex items-center gap-2.5 rounded-md font-semibold tracking-tight">
          <HiveTile className="size-8" />
          <span className="text-lg">Hive</span>
        </Link>
        <nav aria-label="Sections" className="hidden items-center gap-1 md:flex">
          {NAV.map((n) => (
            <Link
              key={n.href}
              href={n.href}
              className="rounded-full px-3.5 py-1.5 text-sm font-medium text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
            >
              {n.label}
            </Link>
          ))}
        </nav>
        <div className="flex items-center gap-2">
          <SignInControl />
        </div>
      </div>
    </header>
  );
}

export function Footer() {
  return (
    <footer className="border-t border-border/70">
      <div className="mx-auto flex max-w-6xl flex-col gap-6 px-5 py-10 text-sm text-muted-foreground sm:flex-row sm:items-center sm:justify-between sm:px-8">
        <div className="flex items-center gap-2.5">
          <HiveTile className="size-7" />
          <span>
            <span className="font-medium text-foreground">Hive</span> · hivefleet.dev
          </span>
        </div>
        <nav aria-label="Footer" className="flex flex-wrap items-center gap-x-5 gap-y-2">
          <Link href="/#what" className="hover:text-foreground">
            What it does
          </Link>
          <Link href="/#how" className="hover:text-foreground">
            How it works
          </Link>
          <Link href="/#security" className="hover:text-foreground">
            Yours
          </Link>
          <Link href="/sign-in/" className="hover:text-foreground">
            Sign in
          </Link>
          <a href="https://www.lamplitlabs.com" className="hover:text-foreground">
            Lamplit Labs
          </a>
        </nav>
      </div>
    </footer>
  );
}
