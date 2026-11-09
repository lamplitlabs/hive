// The lucide glyph for each nav icon name (nav-items.ts keeps names so the
// model stays pure). One place to change a page's icon.

import {
  BookOpenIcon,
  BrainIcon,
  ClipboardCheckIcon,
  CalendarClockIcon,
  ChartLineIcon,
  HomeIcon,
  InboxIcon,
  ListTodoIcon,
  type LucideIcon,
  MessageSquareIcon,
  PlugIcon,
  ScrollTextIcon,
  ServerIcon,
  SettingsIcon,
  VoteIcon,
} from "lucide-react";
import type { NavIcon } from "./nav-items";

export const NAV_ICON: Record<NavIcon, LucideIcon> = {
  home: HomeIcon,
  inbox: InboxIcon,
  sessions: MessageSquareIcon,
  tasks: ListTodoIcon,
  records: BookOpenIcon,
  reports: ClipboardCheckIcon,
  votes: VoteIcon,
  memory: BrainIcon,
  runs: ScrollTextIcon,
  machines: ServerIcon,
  connections: PlugIcon,
  automations: CalendarClockIcon,
  insights: ChartLineIcon,
  settings: SettingsIcon,
};
