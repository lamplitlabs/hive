// The project's website, and where a documentation file is on it. Every
// file under docs/ in Hive's repository is a page there: docs/landing.md is
// https://hivefleet.dev/docs/landing/, with the path in lowercase and any
// anchor kept as GitHub slugs it. The console sends the reader there for
// documentation; source code, issues and releases stay on GitHub. Pure, so
// node --test can check it (site.test.mjs).

export const SITE_URL = "https://hivefleet.dev";

// A link to a file in Hive's repository on GitHub: .../blob/<ref>/<path>.
const REPO_FILE = /^https?:\/\/(?:www\.)?github\.com\/lamplitlabs\/hive\/(?:blob|tree)\/[^/\s?#]+\/([^\s]*)$/i;

// The page of a documentation file. `ref` names the file the way the
// repository does - "docs/landing.md", "landing.md" or "landing", with an
// optional "#anchor" - or is a GitHub link to it. Any other address (a
// GitHub link to source, an issue, a release) comes back as it is.
export function docUrl(ref: string): string {
  let r = (ref || "").trim();
  const repo = REPO_FILE.exec(r);
  if (repo) {
    if (!/^docs(\/|$|[?#])/i.test(repo[1] ?? "")) return r;
    r = repo[1] ?? "";
  } else if (/^[a-z][a-z0-9+.-]*:/i.test(r)) {
    return r;
  }
  const hashAt = r.indexOf("#");
  const anchor = hashAt >= 0 ? r.slice(hashAt + 1) : "";
  const path = (hashAt >= 0 ? r.slice(0, hashAt) : r)
    .replace(/\?.*$/, "")
    .replace(/^(\.?\/)+/, "")
    .replace(/^docs(\/|$)/i, "")
    .replace(/\.md$/i, "")
    .replace(/\/+$/, "");
  const page = path
    .toLowerCase()
    .split("/")
    .filter(Boolean)
    .map(encodeURIComponent)
    .join("/");
  return SITE_URL + "/docs/" + (page ? page + "/" : "") + (anchor ? "#" + anchor : "");
}

export interface TextPart {
  text: string;
  // Set when the part names a documentation page: where it is on the site.
  href?: string;
}

// A GitHub link to a file of Hive's docs, or a file directly under docs/
// named in passing ("docs/security.md", "docs/release.md#channels"). A file
// in a folder of docs/ is not taken from plain text: "docs/context/
// environment.md" or "docs/evolution/constitution.md" in a sentence is a
// file of the reader's own project.
const MENTION = /https?:\/\/(?:www\.)?github\.com\/lamplitlabs\/hive\/(?:blob|tree)\/[^/\s?#]+\/docs\/[^\s)\]>"'`,;]*[^\s)\]>"'`,;.:!?]|docs\/[A-Za-z0-9][A-Za-z0-9_-]*\.md(?:#[A-Za-z0-9_-]+)?(?![\w/-])/gi;
// What may not stand right before a bare "docs/...": a letter, a digit or a
// path character would make it the tail of another name ("mydocs/a.md",
// "project/docs/a.md").
const JOINED = /[\w./-]/;

// A text of Hive's own (a setting's description) cut into plain parts and
// the parts that name a documentation page, each with its address on the
// site. A text that names none comes back as one part.
export function linkDocs(text: string): TextPart[] {
  const t = text || "";
  const parts: TextPart[] = [];
  let at = 0;
  for (const m of t.matchAll(MENTION)) {
    const i = m.index ?? 0;
    if (!/^https?:/i.test(m[0]) && i > 0 && JOINED.test(t[i - 1] ?? "")) continue;
    if (i > at) parts.push({ text: t.slice(at, i) });
    parts.push({ text: m[0], href: docUrl(m[0]) });
    at = i + m[0].length;
  }
  if (at < t.length || !parts.length) parts.push({ text: t.slice(at) });
  return parts;
}
