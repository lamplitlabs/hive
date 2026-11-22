import type { NextConfig } from "next";

// Static export: Vercel serves out/ as plain files, so the site has no server,
// no middleware and no secrets. Everything a page shows is read at build time
// from this folder and from the repository's docs/ (../../docs).
const nextConfig: NextConfig = {
  output: "export",
  trailingSlash: true,
  images: { unoptimized: true },
  reactStrictMode: true,
  // `next dev` would otherwise write AGENTS.md and CLAUDE.md here; agent instructions are store records, never files in the tree.
  agentRules: false,
};

export default nextConfig;
