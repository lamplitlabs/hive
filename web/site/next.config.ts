import type { NextConfig } from "next";

// Static export: Vercel serves out/ as plain files, so the site has no server
// and no middleware. Clerk runs in the browser only (ClerkProvider with the
// publishable key baked in at build time) and the console it leads to is the
// team's own front, never this site.
const nextConfig: NextConfig = {
  output: "export",
  trailingSlash: true,
  images: { unoptimized: true },
  reactStrictMode: true,
  // `next dev` would otherwise write AGENTS.md and CLAUDE.md here; agent instructions are store records, never files in the tree.
  agentRules: false,
};

export default nextConfig;
