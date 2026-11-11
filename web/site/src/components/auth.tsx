"use client";

// Clerk for a static export: the provider runs in the browser only and the
// publishable key is baked in at build time. Without a key (a build with no
// .env) the children render on their own and every sign-in control says the
// sign-in is not set up yet, so the page never breaks for want of a secret
// that is not one.
import { ClerkProvider } from "@clerk/clerk-react";
import type { ReactNode } from "react";
import { CLERK_PUBLISHABLE_KEY, CONSOLE_URL, SIGN_IN_READY } from "@/lib/site";

const appearance = {
  cssLayerName: "clerk",
  variables: {
    colorPrimary: "var(--primary)",
    colorPrimaryForeground: "var(--primary-foreground)",
    colorBackground: "var(--card)",
    colorForeground: "var(--foreground)",
    colorMutedForeground: "var(--muted-foreground)",
    colorInput: "var(--card)",
    colorInputForeground: "var(--foreground)",
    colorBorder: "var(--border)",
    colorRing: "var(--teal)",
    colorNeutral: "var(--foreground)",
    borderRadius: "0.75rem",
    fontFamily: "var(--font-ui)",
    fontFamilyButtons: "var(--font-ui)",
  },
  elements: {
    cardBox: "shadow-pop",
    footer: "hidden",
  },
} as const;

export function Auth({ children }: { children: ReactNode }) {
  if (!SIGN_IN_READY) return <>{children}</>;
  return (
    <ClerkProvider
      publishableKey={CLERK_PUBLISHABLE_KEY}
      appearance={appearance}
      signInUrl="/sign-in/"
      signUpUrl="/sign-up/"
      signInFallbackRedirectUrl={CONSOLE_URL || "/"}
      signUpFallbackRedirectUrl={CONSOLE_URL || "/"}
      afterSignOutUrl="/"
    >
      {children}
    </ClerkProvider>
  );
}
