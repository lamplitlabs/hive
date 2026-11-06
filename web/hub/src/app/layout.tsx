import type { Metadata, Viewport } from "next";
import "./globals.css";
import { AppShell } from "@/components/shell/app-shell";
import { Providers } from "@/components/shell/providers";
import { APPEARANCE_BOOT_SCRIPT } from "@/components/shell/appearance-state";
import { SIDEBAR_BOOT_SCRIPT } from "@/components/shell/sidebar-state";

export const metadata: Metadata = {
  title: { default: "PULSE", template: "%s · PULSE" },
  description: "Your console for PULSE: what waits for your answer, the tasks you gave, and what your agents are doing on every project on this machine.",
  applicationName: "PULSE hub",
};

export const viewport: Viewport = {
  width: "device-width",
  initialScale: 1,
  viewportFit: "cover",
  themeColor: [
    { media: "(prefers-color-scheme: light)", color: "#f9fafb" },
    { media: "(prefers-color-scheme: dark)", color: "#0d1014" },
  ],
};

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en" suppressHydrationWarning className="h-full scroll-smooth antialiased">
      <head>
        {/* Geist Sans and Geist Mono are vendored in public/fonts (OFL) so the export needs no network at build or run time; Atkinson Hyperlegible (the Readable font option) loads on demand through its @font-face. */}
        <link rel="preload" href="/fonts/geist-sans-variable.woff2" as="font" type="font/woff2" crossOrigin="anonymous" />
        <link rel="preload" href="/fonts/geist-mono-variable.woff2" as="font" type="font/woff2" crossOrigin="anonymous" />
        {/* The sidebar's open/collapsed choice and the appearance choices (density, font, text size) go on <html> before the first paint, the way next-themes sets the theme, so a reload never folds or reflows in front of you. The hub's CSP allows inline scripts. */}
        <script dangerouslySetInnerHTML={{ __html: SIDEBAR_BOOT_SCRIPT }} />
        <script dangerouslySetInnerHTML={{ __html: APPEARANCE_BOOT_SCRIPT }} />
      </head>
      <body className="flex min-h-full flex-col bg-background text-foreground">
        <Providers>
          <AppShell>{children}</AppShell>
        </Providers>
      </body>
    </html>
  );
}
