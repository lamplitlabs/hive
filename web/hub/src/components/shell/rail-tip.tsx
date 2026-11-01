"use client";

import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { useSidebarCollapsed } from "./use-sidebar";

// On the sidebar's rail every item is an icon: its name (and the page's hint)
// shows beside it on hover and focus. Open, the items carry their own labels
// and the tooltip stays out of the way. The child must take a ref (a Link or
// a Button), as Radix attaches the trigger to it.
export function RailTip({ label, hint, children }: { label: string; hint?: string; children: React.ReactNode }) {
  const [collapsed] = useSidebarCollapsed();
  return (
    <Tooltip>
      <TooltipTrigger asChild>{children}</TooltipTrigger>
      {collapsed ? (
        <TooltipContent side="right" sideOffset={10}>
          {hint ? (
            <span className="grid gap-0.5">
              <span className="font-medium">{label}</span>
              <span className="text-background/70">{hint}</span>
            </span>
          ) : (
            label
          )}
        </TooltipContent>
      ) : null}
    </Tooltip>
  );
}
