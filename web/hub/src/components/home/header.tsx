"use client";

import Link from "next/link";
import { useEffect, useState } from "react";
import { MessageSquarePlusIcon, PlusIcon } from "lucide-react";
import { Button } from "@/components/ui/button";
import { NewTaskDialog } from "@/components/tasks/task-dialogs";
import { hrefs } from "@/lib/nav";
import { dateLine, greetingLine } from "./home-state";

// Home's header row (spec "Home" 1): "Good morning, Manish" at 32 px, today's
// date under it, and the two things the owner starts from here - a task for
// the fleet or a session with an agent. No headline sentence, no mascot: the
// waiting count lives in the sidebar and on For you.
//
// The greeting and the date depend on the visitor's clock, which the static
// export does not know: both render as a neutral "Hello" first and settle
// after hydration, so the server and the first client paint agree.
export function HomeHeader({ name }: { name?: string | null }) {
  const [clock, setClock] = useState<{ greeting: string; date: string } | null>(null);
  const [newTask, setNewTask] = useState(false);
  useEffect(() => {
    const tick = () => {
      const now = new Date();
      setClock({ greeting: greetingLine(now.getHours(), name), date: dateLine(now) });
    };
    tick();
    // A tab left open overnight greets the next morning correctly.
    const id = window.setInterval(tick, 60_000);
    return () => window.clearInterval(id);
  }, [name]);
  const greeting = clock?.greeting ?? ("Hello" + ((name || "").trim() ? ", " + (name || "").trim() : ""));
  return (
    <header className="flex min-w-0 flex-wrap items-start justify-between gap-x-6 gap-y-4 pt-5 sm:pt-6">
      <div className="min-w-0">
        <h1 className="truncate text-[32px] font-semibold leading-10 tracking-tight">{greeting}</h1>
        <p className="mt-1 text-sm leading-5 text-muted-foreground" suppressHydrationWarning>
          {clock?.date ?? "\u00a0"}
        </p>
      </div>
      <div className="flex shrink-0 items-center gap-2">
        <Button variant="outline" asChild>
          <Link href={hrefs.conversations()}>
            <MessageSquarePlusIcon /> New session
          </Link>
        </Button>
        <Button onClick={() => setNewTask(true)}>
          <PlusIcon /> New task
        </Button>
      </div>
      <NewTaskDialog open={newTask} onOpenChange={setNewTask} />
    </header>
  );
}
