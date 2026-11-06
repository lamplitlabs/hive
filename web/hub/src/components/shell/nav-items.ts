// The sidebar's navigation, as data (spec "App shell" - Navigation in four
// groups). Pure: icons are named, not imported, so node --test can check the
// model and app-shell.tsx maps the names to lucide glyphs. The labels are what
// each page is about, not what the store calls it (Runs is the harness's log,
// Machines is compute).

export type NavIcon =
  | "home"
  | "inbox"
  | "sessions"
  | "tasks"
  | "records"
  | "votes"
  | "memory"
  | "runs"
  | "machines"
  | "connections"
  | "automations"
  | "insights"
  | "settings";

export type NavItem = {
  href: string;
  label: string;
  hint: string;
  icon: NavIcon;
  /** The G-chord's second key ("H" for G then H), for the four pages that have one. */
  shortcut?: string;
  match: (pathname: string) => boolean;
};

export type NavGroup = { label: string | null; items: NavItem[] };

const under = (prefix: string) => (p: string) => p === prefix || p.startsWith(prefix + "/") || p.startsWith(prefix + "?");

export const NAV_GROUPS: NavGroup[] = [
  {
    label: null,
    items: [
      { href: "/", label: "Home", hint: "What needs you, what is running, whether anything is broken", icon: "home", shortcut: "H", match: (p) => p === "/" || p === "" },
      { href: "/for-you/", label: "For you", hint: "Everything that waits on your answer, across projects", icon: "inbox", shortcut: "F", match: under("/for-you") },
      { href: "/chat/", label: "Sessions", hint: "Ask an agent to work, watch it, answer its questions", icon: "sessions", shortcut: "S", match: under("/chat") },
      { href: "/tasks/", label: "Tasks", hint: "The work you gave your agents", icon: "tasks", shortcut: "T", match: under("/tasks") },
    ],
  },
  {
    label: "Knowledge",
    items: [
      { href: "/records/", label: "Records", hint: "Decisions, plans, guides and notes", icon: "records", match: (p) => under("/records")(p) || under("/code-deliveries")(p) },
      { href: "/votes/", label: "Votes", hint: "Ideas your agents are voting on", icon: "votes", match: under("/votes") },
      { href: "/memory/", label: "Memory", hint: "What agents remembered across runs, file by file", icon: "memory", match: under("/memory") },
    ],
  },
  {
    label: "System",
    items: [
      { href: "/harness/", label: "Runs", hint: "Every agent run, step by step", icon: "runs", match: under("/harness") },
      { href: "/compute/", label: "Machines", hint: "Where agents run, and requests for more", icon: "machines", match: under("/compute") },
      { href: "/connections/", label: "Connections", hint: "Tools and skills agents can use", icon: "connections", match: under("/connections") },
      { href: "/automations/", label: "Automations", hint: "Standing jobs on a schedule, kept in each project's PULSE file", icon: "automations", match: under("/automations") },
      { href: "/insights/", label: "Insights", hint: "How your projects are doing over time", icon: "insights", match: under("/insights") },
    ],
  },
];

// Settings sits in the owner row, not in a group, but the phone's More sheet
// and the page titles want it in the same shape.
export const SETTINGS_ITEM: NavItem = { href: "/settings/", label: "Settings", hint: "How PULSE behaves on this machine", icon: "settings", match: under("/settings") };

export const NAV_ITEMS: NavItem[] = NAV_GROUPS.flatMap((g) => g.items);

// The first group's four pages make the phone's bottom bar; the rest go under More.
export const PRIMARY_ITEMS: NavItem[] = NAV_GROUPS[0].items;
export const MORE_GROUPS: NavGroup[] = NAV_GROUPS.slice(1);

export function activeItem(pathname: string, items: NavItem[] = [...NAV_ITEMS, SETTINGS_ITEM]): NavItem | undefined {
  return items.find((n) => n.match(pathname));
}

// "G then H": what a tooltip says under the item's name.
export function shortcutWords(item: Pick<NavItem, "shortcut">): string | null {
  return item.shortcut ? "G then " + item.shortcut : null;
}

// The machine's name, as the sidebar header shows it: `org.host` is the raw
// host name ("manishs-mac", already without ".local" - but strip one anyway);
// dashes read as spaces. Empty or unknown falls back to "This machine".
export function machineName(host: string | null | undefined): string {
  const raw = (host || "").trim().replace(/\.local$/i, "").replace(/-+/g, " ").replace(/\s+/g, " ").trim();
  return raw || "This machine";
}
