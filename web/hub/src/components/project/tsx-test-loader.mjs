// Node ESM hooks that let the hub's node:test suites import the app's .tsx
// components (and their "@/…" aliases) directly, the way Next does: .ts/.tsx
// under src/ are transpiled with the TypeScript already in node_modules, and
// the two next/* modules the components touch are replaced by tiny stubs that
// render anchors and inert router hooks. Test files register this loader with
// module.register() before dynamically importing the component under test.
import { readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const SRC = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..") + path.sep;

const STUBS = {
  "next/link":
    "import React from 'react';\n" +
    "export default function Link({ href, children, ...rest }) {\n" +
    "  return React.createElement('a', { href: typeof href === 'string' ? href : '#', ...rest }, children);\n" +
    "}\n",
  "next/navigation":
    "export function useRouter() { return { push() {}, replace() {}, refresh() {}, back() {}, forward() {}, prefetch() {} }; }\n" +
    "export function usePathname() { return '/'; }\n" +
    "export function useSearchParams() { return new URLSearchParams(); }\n" +
    "export function useParams() { return {}; }\n" +
    "export function redirect() {}\n",
};

// Stubs resolve as virtual files beside this loader so their own bare imports
// (react) resolve against the hub's node_modules.
const stubUrl = (name) => pathToFileURL(path.join(SRC, "components", "project", ".stub-" + name.replaceAll("/", "-") + ".mjs")).href;

export async function resolve(specifier, context, nextResolve) {
  if (Object.hasOwn(STUBS, specifier)) return { url: stubUrl(specifier), shortCircuit: true };
  let spec = specifier;
  const aliased = spec.startsWith("@/");
  if (aliased) spec = pathToFileURL(path.join(SRC, spec.slice(2))).href;
  if (aliased || spec.startsWith(".") || spec.startsWith("file:")) {
    // TS sources import extensionless; try the resolvable candidates.
    for (const cand of [spec, spec + ".ts", spec + ".tsx", spec + "/index.ts", spec + "/index.tsx"]) {
      try {
        return await nextResolve(cand, context);
      } catch {
        // try the next candidate
      }
    }
  }
  // Bare package specifiers keep the default resolver (package exports).
  return nextResolve(specifier, context);
}

export async function load(url, context, nextLoad) {
  if (url.includes("/.stub-")) {
    const name = url.slice(url.indexOf("/.stub-") + "/.stub-".length, -".mjs".length).replace("-", "/");
    return { format: "module", source: STUBS[name], shortCircuit: true };
  }
  if (url.endsWith(".ts") && url.includes("/src/")) {
    // Plain .ts goes through node's own type stripping, which keeps every
    // offset, so coverage of a module loaded after this loader registers
    // (one process under --experimental-test-isolation=none) still lines up
    // with its source. transpileModule's shorter output would leave the
    // module's tail counted as uncovered (and its lines shifted). Syntax
    // stripping cannot erase (enums, namespaces) falls through to the
    // transpile below. Like a test importing a .ts directly, a stripped module
    // must import its types with `import type`.
    try {
      return await nextLoad(url, context);
    } catch (e) {
      if (e?.code !== "ERR_UNSUPPORTED_TYPESCRIPT_SYNTAX") throw e;
    }
  }
  if (url.endsWith(".tsx") || (url.endsWith(".ts") && url.includes("/src/"))) {
    // TypeScript is loaded only here, for a .tsx or a .ts that stripping
    // cannot run: a suite that reaches plain .ts modules alone (inbox-state,
    // job-repair) then runs in a clone without node_modules too, the way
    // pulse evolve measure scores the hub's coverage fixtures.
    const { default: ts } = await import("typescript");
    const source = await readFile(fileURLToPath(url), "utf8");
    const out = ts.transpileModule(source, {
      compilerOptions: {
        jsx: ts.JsxEmit.ReactJSX,
        module: ts.ModuleKind.ESNext,
        target: ts.ScriptTarget.ES2022,
        moduleResolution: ts.ModuleResolutionKind.Bundler,
      },
      fileName: fileURLToPath(url),
    });
    return { format: "module", source: out.outputText, shortCircuit: true };
  }
  return nextLoad(url, context);
}
