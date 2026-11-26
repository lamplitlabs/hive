// What search engines and link previews read: each page's metadata and the
// JSON-LD beside it. Pure, so the tests check every title, description and
// structured-data object without a browser.
import type { Metadata } from "next";
import { absoluteUrl, SITE } from "./site.ts";

/** The social card (scripts/brand.py draws it): 1200 x 630. */
export const OG_IMAGE = {
  url: "/og.png",
  width: 1200,
  height: 630,
  type: "image/png",
  alt: "The Hive mark, a hexagon cut into three brackets around a core with a node in each gap, beside the word HIVE and the line: the agent harness that improves your projects, and itself.",
} as const;

export const TITLE_MAX = 60;
export const DESCRIPTION_MIN = 70;
export const DESCRIPTION_MAX = 160;

export type PageSeo = {
  path: string;
  /** The whole <title>. */
  title: string;
  description: string;
  /** The title a link preview shows, when it may be longer than the tab's. */
  shareTitle?: string;
};

/** One page's metadata: title, description, canonical address, Open Graph and the large Twitter card. */
export function pageMetadata(page: PageSeo, type: "website" | "article" = "website"): Metadata {
  const title = page.shareTitle ?? page.title;
  return {
    title: { absolute: page.title },
    description: page.description,
    alternates: { canonical: page.path },
    openGraph: {
      type,
      siteName: SITE.name,
      locale: "en_US",
      url: page.path,
      title,
      description: page.description,
      images: [OG_IMAGE],
    },
    twitter: {
      card: "summary_large_image",
      title,
      description: page.description,
      images: [{ url: OG_IMAGE.url, alt: OG_IMAGE.alt }],
    },
  };
}

/** The Google Search Console token, when the build was given one. */
export function googleVerification(env: Record<string, string | undefined> = process.env): string | undefined {
  const token = (env.NEXT_PUBLIC_GOOGLE_SITE_VERIFICATION ?? "").trim();
  return token || undefined;
}

const ORG_ID = `${SITE.url}/#organization`;
const SITE_ID = `${SITE.url}/#website`;
const APP_ID = `${SITE.url}/#software`;
const MIT = "https://opensource.org/license/mit";

type Json = Record<string, unknown>;

/** On every page: the site and who publishes it. */
export function siteJsonLd(): Json {
  return {
    "@context": "https://schema.org",
    "@graph": [
      {
        "@type": "WebSite",
        "@id": SITE_ID,
        url: `${SITE.url}/`,
        name: SITE.name,
        alternateName: SITE.domain,
        description: SITE.sentence,
        inLanguage: "en",
        publisher: { "@id": ORG_ID },
      },
      {
        "@type": "Organization",
        "@id": ORG_ID,
        name: SITE.holder,
        url: SITE.holderUrl,
        sameAs: ["https://github.com/lamplitlabs"],
      },
    ],
  };
}

/** On the landing page: the tool itself, and where its source lives. */
export function softwareJsonLd(): Json {
  return {
    "@context": "https://schema.org",
    "@graph": [
      {
        "@type": "SoftwareApplication",
        "@id": APP_ID,
        name: SITE.name,
        description: SITE.sentence,
        url: `${SITE.url}/`,
        image: absoluteUrl(OG_IMAGE.url),
        applicationCategory: "DeveloperApplication",
        operatingSystem: "macOS, Linux",
        offers: { "@type": "Offer", price: "0", priceCurrency: "USD" },
        isAccessibleForFree: true,
        license: MIT,
        installUrl: absoluteUrl("/install/"),
        softwareHelp: { "@type": "CreativeWork", name: "Hive documentation", url: absoluteUrl("/docs/") },
        sameAs: [SITE.repo],
        author: { "@id": ORG_ID },
        publisher: { "@id": ORG_ID },
      },
      {
        "@type": "SoftwareSourceCode",
        name: `${SITE.name} source code`,
        codeRepository: SITE.repo,
        programmingLanguage: "Rust",
        license: MIT,
        targetProduct: { "@id": APP_ID },
      },
    ],
  };
}

export type Faq = { q: string; a: string };

/** The questions a page answers, as it answers them. */
export function faqJsonLd(faqs: readonly Faq[]): Json {
  return {
    "@context": "https://schema.org",
    "@type": "FAQPage",
    mainEntity: faqs.map((f) => ({
      "@type": "Question",
      name: f.q,
      acceptedAnswer: { "@type": "Answer", text: f.a },
    })),
  };
}

export type Crumb = { name: string; path: string };

export function breadcrumbJsonLd(crumbs: readonly Crumb[]): Json {
  return {
    "@context": "https://schema.org",
    "@type": "BreadcrumbList",
    itemListElement: crumbs.map((c, i) => ({
      "@type": "ListItem",
      position: i + 1,
      name: c.name,
      item: absoluteUrl(c.path),
    })),
  };
}

/** One page of the documentation. */
export function techArticleJsonLd(doc: { path: string; title: string; description: string }): Json {
  const url = absoluteUrl(doc.path);
  return {
    "@context": "https://schema.org",
    "@type": "TechArticle",
    headline: doc.title.length > 110 ? `${doc.title.slice(0, 109).trimEnd()}…` : doc.title,
    description: doc.description,
    url,
    mainEntityOfPage: { "@type": "WebPage", "@id": url },
    image: absoluteUrl(OG_IMAGE.url),
    inLanguage: "en",
    isPartOf: { "@id": SITE_ID },
    author: { "@id": ORG_ID },
    publisher: { "@id": ORG_ID },
  };
}

/** JSON-LD as the text of a script element: a `<` can never close the script early. */
export function serializeJsonLd(data: unknown): string {
  return JSON.stringify(data).replace(/</g, "\\u003c");
}
