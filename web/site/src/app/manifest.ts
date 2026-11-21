import type { MetadataRoute } from "next";
import { PAGES, SITE } from "@/lib/site";

export const dynamic = "force-static";

// /manifest.webmanifest: the name, the night field's two colours and the touch icons (scripts/brand.py draws them).
export default function manifest(): MetadataRoute.Manifest {
  return {
    name: PAGES.home.title,
    short_name: SITE.name,
    description: PAGES.home.description,
    start_url: "/",
    scope: "/",
    display: "minimal-ui",
    lang: "en",
    theme_color: "#0c1530",
    background_color: "#060a14",
    icons: [
      { src: "/icon-192.png", sizes: "192x192", type: "image/png", purpose: "any" },
      { src: "/icon-512.png", sizes: "512x512", type: "image/png", purpose: "any" },
    ],
  };
}
