import type { Metadata, Viewport } from "next";
import "./globals.css";
import { googleVerification } from "@/lib/seo";
import { PAGES, SITE } from "@/lib/site";

const google = googleVerification();

// What every page shares. Each page brings its own title, description,
// canonical address and link preview (pageMetadata in src/lib/seo.ts); the
// 404 page brings none of the last two, which is why they are not set here.
export const metadata: Metadata = {
  metadataBase: new URL(SITE.url),
  title: { default: PAGES.home.title, template: `%s · ${SITE.name}` },
  description: PAGES.home.description,
  applicationName: SITE.name,
  authors: [{ name: SITE.holder, url: SITE.holderUrl }],
  creator: SITE.holder,
  publisher: SITE.holder,
  robots: { index: true, follow: true },
  formatDetection: { telephone: false, email: false, address: false },
  ...(google ? { verification: { google } } : {}),
};

export const viewport: Viewport = {
  width: "device-width",
  initialScale: 1,
  colorScheme: "light dark",
  // The header sits on the night field in both schemes.
  themeColor: "#0c1530",
};

export default function RootLayout({ children }: Readonly<{ children: React.ReactNode }>) {
  return (
    <html lang="en" data-scroll-behavior="smooth">
      <head>
        <link rel="preload" href="/fonts/geist-sans-latin.woff2" as="font" type="font/woff2" crossOrigin="anonymous" />
      </head>
      <body className="flex min-h-dvh flex-col">{children}</body>
    </html>
  );
}
