import type { MetadataRoute } from "next";
import { sitemapEntries } from "@/lib/pages";

export const dynamic = "force-static";

// Every page the site exports: the static pages and one per file of the repository's docs/.
export default function sitemap(): MetadataRoute.Sitemap {
  return sitemapEntries();
}
