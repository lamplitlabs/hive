import { loadDocs, searchIndex } from "@/lib/docs";

// The search box's index: the site's pages and every docs page with its sections, written once at build time as a static file.
export const dynamic = "force-static";

export function GET() {
  return Response.json(searchIndex(loadDocs()));
}
