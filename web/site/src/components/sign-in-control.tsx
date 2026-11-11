"use client";

// The sign-in control in the header: "Sign in" for a visitor, the member's
// avatar once they are in. With no Clerk key in the build it is a quiet link
// to the sign-in page, which explains.
import { SignedIn, SignedOut, UserButton } from "@clerk/clerk-react";
import Link from "next/link";
import { CONSOLE_URL, SIGN_IN_READY } from "@/lib/site";

const link = "inline-flex h-9 items-center rounded-full px-4 text-sm font-medium text-foreground/80 transition-colors hover:bg-muted hover:text-foreground";
const primary =
  "inline-flex h-9 items-center gap-1.5 rounded-full bg-primary px-4 text-sm font-medium text-primary-foreground shadow-card transition-colors hover:bg-primary-strong";

export function SignInControl() {
  if (!SIGN_IN_READY) {
    return (
      <Link href="/sign-in/" className={primary}>
        Sign in
      </Link>
    );
  }
  return (
    <>
      <SignedOut>
        <Link href="/sign-in/" className={primary}>
          Sign in
        </Link>
      </SignedOut>
      <SignedIn>
        {CONSOLE_URL ? (
          <a href={CONSOLE_URL} className={link}>
            Open your console
          </a>
        ) : null}
        <UserButton />
      </SignedIn>
    </>
  );
}
