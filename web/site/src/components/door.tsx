"use client";

// The two doors: Clerk's SignIn or SignUp when the build carries a key, a
// plain explanation when it does not. Hash routing keeps /sign-in/ and
// /sign-up/ as ordinary static pages (the export has no server to route
// /sign-in/factor-one), and Clerk puts its steps after the # instead.
import { SignIn, SignUp } from "@clerk/clerk-react";
import Link from "next/link";
import { HiveTile } from "@/components/hive-mark";
import { CONSOLE_URL, SIGN_IN_READY } from "@/lib/site";

export function Door({ kind }: { kind: "sign-in" | "sign-up" }) {
  return (
    <section className="surface-hero flex-1">
      <div className="mx-auto flex max-w-6xl flex-col items-center px-5 py-14 sm:px-8 md:py-20">
        <HiveTile className="size-12" />
        <h1 className="mt-5 text-balance text-center text-3xl font-semibold tracking-tight sm:text-4xl">
          {kind === "sign-in" ? "Sign in to your team" : "Create your account"}
        </h1>
        <p className="mt-3 max-w-md text-center text-pretty text-muted-foreground">
          {kind === "sign-in"
            ? "One login for every machine your team runs Hive on. Your console is the team's own; this page only opens the door."
            : "An account here is the login every machine in your team inherits. Your team's owner adds you to the team afterwards."}
        </p>
        <div className="mt-10 w-full max-w-md">
          {SIGN_IN_READY ? (
            kind === "sign-in" ? (
              <SignIn routing="hash" signUpUrl="/sign-up/" fallbackRedirectUrl={CONSOLE_URL || "/"} />
            ) : (
              <SignUp routing="hash" signInUrl="/sign-in/" fallbackRedirectUrl={CONSOLE_URL || "/"} />
            )
          ) : (
            <NotReady />
          )}
        </div>
        <p className="mt-8 text-sm text-muted-foreground">
          {kind === "sign-in" ? (
            <>
              No account yet?{" "}
              <Link href="/sign-up/" className="text-link underline-offset-4 hover:underline">
                Create one
              </Link>
            </>
          ) : (
            <>
              Already a member?{" "}
              <Link href="/sign-in/" className="text-link underline-offset-4 hover:underline">
                Sign in
              </Link>
            </>
          )}
        </p>
      </div>
    </section>
  );
}

function NotReady() {
  return (
    <div className="rounded-card border border-border bg-card p-6 text-sm leading-relaxed shadow-card">
      <p className="font-medium">The sign-in is not set up on this copy of the site.</p>
      <p className="mt-2 text-muted-foreground">
        This build has no <code className="font-mono text-[13px]">NEXT_PUBLIC_CLERK_PUBLISHABLE_KEY</code>, so there is
        nothing to sign in to here. The one at{" "}
        <a href="https://hivefleet.dev/sign-in/" className="text-link underline-offset-4 hover:underline">
          hivefleet.dev
        </a>{" "}
        has it. To run your own, copy <code className="font-mono text-[13px]">.env.example</code> to{" "}
        <code className="font-mono text-[13px]">.env.local</code> with your Clerk key and build again.
      </p>
    </div>
  );
}
