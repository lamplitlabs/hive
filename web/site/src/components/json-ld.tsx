import { serializeJsonLd } from "@/lib/seo";

/** Structured data for search engines, written into the page as it is built. */
export function JsonLd({ data }: { data: unknown }) {
  return <script type="application/ld+json" dangerouslySetInnerHTML={{ __html: serializeJsonLd(data) }} />;
}
