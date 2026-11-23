import { loadDocs, searchIndex } from "@/lib/docs";

// The search box's index: every docs page's title and headings, written once at build time as a static file.
export const dynamic = "force-static";

export function GET() {
  return Response.json(searchIndex(loadDocs()));
}
