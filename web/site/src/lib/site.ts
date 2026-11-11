// What the site knows about where it runs. Read at build time (Next inlines
// NEXT_PUBLIC_* into the export), so a deploy with another key is another
// build. Pure, so the tests can import it.

/** Clerk's publishable key; empty when the build had none. */
export const CLERK_PUBLISHABLE_KEY = (process.env.NEXT_PUBLIC_CLERK_PUBLISHABLE_KEY ?? "").trim();

/** Where a signed-in member's console lives (the team front); empty until the front exists. */
export const CONSOLE_URL = (process.env.NEXT_PUBLIC_HIVE_CONSOLE_URL ?? "").trim();

/** True when the sign-in can work: the build carried a Clerk key. */
export const SIGN_IN_READY = CLERK_PUBLISHABLE_KEY.length > 0;

export const SITE = {
  name: "Hive",
  domain: "hivefleet.dev",
  url: "https://hivefleet.dev",
  tagline: "Coding agents for the whole team",
  description:
    "One engineer with a good agent setup ships fast; the team around them does not. Hive gives a team one shared brain for its coding agents: sign in once, enrol every engineer's machine with one command, and every task, rule, skill and review belongs to the team.",
} as const;
