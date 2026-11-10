import type { NextConfig } from "next";

// Static export: the hive hub embeds web/hub/out (rust-embed in
// crates/hive-hub/src/server.rs; crates/hive-hub/build.rs runs this build,
// so out/ is a build product and never committed) and serves
// out/<route>/index.html. The dev-only rewrite proxies /api to a
// running `hive hub --no-browser --port 8787`; `output: "export"` ignores
// rewrites at build time, which is what we want (every API call is relative).
const hubPort = process.env.HIVE_HUB_PORT ?? "8787";

const dev = process.env.NODE_ENV === "development";

const nextConfig: NextConfig = {
  output: "export",
  trailingSlash: true,
  images: { unoptimized: true },
  reactStrictMode: true,
  // `next dev` would otherwise write AGENTS.md and CLAUDE.md here; agent instructions are store records, never files in the tree.
  agentRules: false,
  ...(dev
    ? {
        // Without this, dev would 308 /api/v1/x -> /api/v1/x/ before proxying.
        skipTrailingSlashRedirect: true,
        async rewrites() {
          return [{ source: "/api/:path*", destination: `http://127.0.0.1:${hubPort}/api/:path*` }];
        },
      }
    : {}),
};

export default nextConfig;
