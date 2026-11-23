import { llmsText } from "@/lib/pages";

// /llms.txt: what Hive is and where every page is, in plain text, for language models that read sites.
export const dynamic = "force-static";

export function GET() {
  return new Response(llmsText(), { headers: { "Content-Type": "text/plain; charset=utf-8" } });
}
