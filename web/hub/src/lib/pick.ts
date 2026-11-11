// Pick & Polish: point at an element in the Browser tab and the agent gets
// told exactly which one, the way the GitHub Copilot app's browser panel
// works. The hub cannot reach into a page from another origin, so a small
// helper script (public/pick.js, served at /pick.js) runs inside the dev page
// and talks to the hub over postMessage; this module is the contract both
// ends keep, plus the text form a pick takes in a message. Pure, so
// components/chat/browser.tsx stays a view and node --test covers it.

/** One element the owner picked on a page. */
export interface ElementPick {
  /** Unique in the composer: the page, the selector and when it was picked. */
  key: string;
  /** The page's address when it was picked (what pick.js saw, after SPA moves). */
  url: string;
  title: string;
  /** A CSS path that finds the element again: `main > form > button.primary`. */
  selector: string;
  tag: string;
  id: string;
  classes: string[];
  /** The element's visible text, trimmed to a line. */
  text: string;
  /** Its outer HTML, cut short. */
  html: string;
  role: string;
  /** aria-label, alt, title, placeholder or name - whichever it had. */
  label: string;
  href: string;
  src: string;
  /** Nearest React components, innermost first, when the page is React in dev. */
  components: string[];
  rect: { x: number; y: number; w: number; h: number };
}

/** What pick.js sends up: it is there (and where the page is), a pick, or a cancel. */
export type PickMessage =
  | { type: "hello"; url: string; title: string }
  | { type: "picked"; pick: ElementPick }
  | { type: "cancel" };

const PREFIX = "hive-pick";

/** The message the hub sends down to start or stop picking. */
export function pickRequest(on: boolean): { type: string; on: boolean } {
  return { type: PREFIX, on };
}

const str = (v: unknown, max = 2000): string =>
  typeof v === "string" ? v.slice(0, max) : "";
const strs = (v: unknown, max = 12): string[] =>
  Array.isArray(v) ? v.filter((x) => typeof x === "string").slice(0, max) : [];
const n = (v: unknown): number =>
  typeof v === "number" && Number.isFinite(v) ? Math.round(v) : 0;

// Reads a message from pick.js, or nothing for anything else the window
// hears (other scripts, extensions, other frames). Every field is checked
// and cut: the page is the agent's work in progress, not something to trust.
export function readPickMessage(data: unknown): PickMessage | null {
  if (!data || typeof data !== "object") return null;
  const d = data as Record<string, unknown>;
  if (d.type === PREFIX + "-hello")
    return { type: "hello", url: str(d.url, 2000), title: str(d.title, 200) };
  if (d.type === PREFIX + "-cancel") return { type: "cancel" };
  if (d.type !== PREFIX + "-picked" || !d.element || typeof d.element !== "object")
    return null;
  const e = d.element as Record<string, unknown>;
  const r = (e.rect && typeof e.rect === "object" ? e.rect : {}) as Record<
    string,
    unknown
  >;
  const url = str(d.url, 2000);
  const selector = str(e.selector, 500);
  const tag = str(e.tag, 40).toLowerCase();
  if (!tag) return null;
  return {
    type: "picked",
    pick: {
      key: url + "|" + selector + "|" + Date.now(),
      url,
      title: str(d.title, 200),
      selector,
      tag,
      id: str(e.id, 120),
      classes: strs(e.classes).map((c) => c.slice(0, 80)),
      text: oneLine(str(e.text, 400), 160),
      html: oneLine(str(e.html, 2000), 600),
      role: str(e.role, 40),
      label: oneLine(str(e.label, 200), 120),
      href: str(e.href, 500),
      src: str(e.src, 500),
      components: strs(e.components, 6).map((c) => c.slice(0, 60)),
      rect: { x: n(r.x), y: n(r.y), w: n(r.w), h: n(r.h) },
    },
  };
}

function oneLine(s: string, max: number): string {
  const t = s.replace(/\s+/g, " ").trim();
  return t.length > max ? t.slice(0, max - 1) + "…" : t;
}

/** `button#save.btn.primary` - the element as a selector-shaped name. */
export function elementName(p: {
  tag: string;
  id: string;
  classes: string[];
}): string {
  const classes = p.classes.slice(0, 3).map((c) => "." + c);
  return p.tag + (p.id ? "#" + p.id : "") + classes.join("");
}

/** The chip's words: the element, then what it says. */
export function pickLabel(p: {
  tag: string;
  id: string;
  classes: string[];
  text: string;
  label: string;
}): string {
  const said = p.text || p.label;
  return said
    ? elementName(p) + " “" + (said.length > 40 ? said.slice(0, 39) + "…" : said) + "”"
    : elementName(p);
}

/** "localhost:3000/settings" for the chip's second line. */
export function pickPlace(url: string): string {
  try {
    const u = new URL(url);
    const path = u.pathname.replace(/\/+$/, "");
    return u.host + (path.length > 32 ? "…" + path.slice(-31) : path);
  } catch {
    return url;
  }
}

// ----- the text form ---------------------------------------------------------

// A pick travels inside the message as a fenced block the agent reads as it
// is and the transcript folds into a chip:
//
//   ```pick
//   url: http://localhost:3000/settings
//   element: button#save.btn.primary
//   text: Save changes
//   components: SaveButton < SettingsForm
//   selector: main > form > button
//   rect: 812,340 160x40
//   html: <button id="save" class="btn primary">Save changes</button>
//   ```
export const PICK_FENCE = "```pick";

export function pickBlock(p: ElementPick): string {
  const lines = ["url: " + p.url, "element: " + elementName(p)];
  if (p.text) lines.push("text: " + p.text);
  if (p.label) lines.push("label: " + p.label);
  if (p.role) lines.push("role: " + p.role);
  if (p.href) lines.push("href: " + p.href);
  if (p.src) lines.push("src: " + p.src);
  if (p.components.length) lines.push("components: " + p.components.join(" < "));
  if (p.selector) lines.push("selector: " + p.selector);
  if (p.rect.w || p.rect.h)
    lines.push("rect: " + p.rect.x + "," + p.rect.y + " " + p.rect.w + "x" + p.rect.h);
  if (p.html) lines.push("html: " + p.html.replace(/```/g, "` ` `"));
  return PICK_FENCE + "\n" + lines.join("\n") + "\n```";
}

/** What a message with picks and no words asks. */
export const PICK_DEFAULT_TEXT = "Polish the element I picked.";

/** The message as sent: the owner's words, then one block per pick. */
export function withPicks(text: string, picks: ElementPick[]): string {
  if (!picks.length) return text;
  const head = text.trim() || PICK_DEFAULT_TEXT;
  return head + "\n\n" + picks.map(pickBlock).join("\n");
}

/** A pick read back out of a message, for the chip in the transcript. */
export interface ParsedPick {
  url: string;
  element: string;
  text: string;
  selector: string;
}

const BLOCK_RE = /```pick\n([\s\S]*?)\n```/g;

/** The owner's words without the pick blocks, and the picks they carried. */
export function splitPicks(message: string): { text: string; picks: ParsedPick[] } {
  const picks: ParsedPick[] = [];
  const text = message
    .replace(BLOCK_RE, (_m, body: string) => {
      const f: Record<string, string> = {};
      for (const line of body.split("\n")) {
        const i = line.indexOf(": ");
        if (i > 0) f[line.slice(0, i)] = line.slice(i + 2);
      }
      if (f.element)
        picks.push({
          url: f.url ?? "",
          element: f.element,
          text: f.text ?? "",
          selector: f.selector ?? "",
        });
      return "";
    })
    .replace(/\n{3,}/g, "\n\n")
    .trim();
  return { text, picks };
}

// ----- the helper's installation ---------------------------------------------

/** The tag a dev page adds so the hub can pick on it; `origin` is the hub's. */
export function pickSnippet(origin: string): string {
  return '<script src="' + origin.replace(/\/+$/, "") + '/pick.js"></script>';
}

/** What the owner asks the agent when the page has no helper yet. */
export function askForHelper(origin: string, url: string): string {
  return (
    "Add the Hive pick helper to this app's development build so I can pick elements on it from the hub's Browser tab: include " +
    pickSnippet(origin) +
    " in the page (" +
    (url || "the app's pages") +
    "), in development only, never in production. It is a plain script tag; nothing else to install."
  );
}
