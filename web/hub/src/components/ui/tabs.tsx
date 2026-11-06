"use client"

import * as React from "react"
import { cva, type VariantProps } from "class-variance-authority"
import { cn } from "@/lib/utils"
import { Tabs as TabsPrimitive } from "radix-ui"

// One look: an underlined tab row for the sections of a page (a project's
// Overview, Work, Board...) and for the views of a list - the active tab has
// a 2px teal rule under it, nothing else. `default` and `line` are both that
// underline (the old segmented `default` is gone; `line` stays as an alias so
// callers need not change). `segmented` keeps a small control on a muted
// track for a chart's range or grouping toggles.
function Tabs({
  className,
  orientation = "horizontal",
  ...props
}: React.ComponentProps<typeof TabsPrimitive.Root>) {
  return (
    <TabsPrimitive.Root
      data-slot="tabs"
      data-orientation={orientation}
      className={cn(
        "group/tabs flex gap-2 data-horizontal:flex-col",
        className
      )}
      {...props}
    />
  )
}

const underlineList =
  "h-auto gap-4 rounded-none border-b border-border bg-transparent p-0 group-data-vertical/tabs:border-r group-data-vertical/tabs:border-b-0"

const tabsListVariants = cva(
  "group/tabs-list inline-flex w-fit items-center justify-center text-muted-foreground group-data-vertical/tabs:h-fit group-data-vertical/tabs:flex-col",
  {
    variants: {
      variant: {
        default: underlineList,
        line: underlineList,
        segmented:
          "rounded-md bg-muted p-0.5 ring-1 ring-border/70 ring-inset group-data-horizontal/tabs:h-9",
      },
    },
    defaultVariants: {
      variant: "default",
    },
  }
)

// The look a variant resolves to; triggers style themselves from it.
type TabsLook = "underline" | "segmented"
function lookOf(variant: VariantProps<typeof tabsListVariants>["variant"]): TabsLook {
  return variant === "segmented" ? "segmented" : "underline"
}

function TabsList({
  className,
  variant = "default",
  ...props
}: React.ComponentProps<typeof TabsPrimitive.List> &
  VariantProps<typeof tabsListVariants>) {
  return (
    <TabsPrimitive.List
      data-slot="tabs-list"
      data-variant={variant}
      data-look={lookOf(variant)}
      className={cn(tabsListVariants({ variant }), className)}
      {...props}
    />
  )
}

function TabsTrigger({
  className,
  ...props
}: React.ComponentProps<typeof TabsPrimitive.Trigger>) {
  return (
    <TabsPrimitive.Trigger
      data-slot="tabs-trigger"
      className={cn(
        "relative inline-flex h-full flex-1 items-center justify-center gap-1.5 border border-transparent text-sm font-medium whitespace-nowrap text-muted-foreground transition-[color,background-color,border-color,box-shadow] duration-150 ease-out-soft outline-none group-data-vertical/tabs:w-full group-data-vertical/tabs:justify-start hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background disabled:pointer-events-none disabled:opacity-50 [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-4",
        // underline: a 2px rule under the active tab (to its right when vertical)
        "group-data-[look=underline]/tabs-list:-mb-px group-data-[look=underline]/tabs-list:rounded-none group-data-[look=underline]/tabs-list:border-0 group-data-[look=underline]/tabs-list:border-b-2 group-data-[look=underline]/tabs-list:px-1 group-data-[look=underline]/tabs-list:pt-1 group-data-[look=underline]/tabs-list:pb-2 group-data-[look=underline]/tabs-list:data-active:border-link group-data-[look=underline]/tabs-list:data-active:bg-transparent group-data-[look=underline]/tabs-list:data-active:text-foreground group-data-[look=underline]/tabs-list:data-active:shadow-none",
        "group-data-vertical/tabs:group-data-[look=underline]/tabs-list:-mr-px group-data-vertical/tabs:group-data-[look=underline]/tabs-list:mb-0 group-data-vertical/tabs:group-data-[look=underline]/tabs-list:border-b-0 group-data-vertical/tabs:group-data-[look=underline]/tabs-list:border-r-2 group-data-vertical/tabs:group-data-[look=underline]/tabs-list:py-1.5 group-data-vertical/tabs:group-data-[look=underline]/tabs-list:pr-3",
        // segmented: a chip on a muted track, the active one lifted to card
        "group-data-[look=segmented]/tabs-list:rounded-sm group-data-[look=segmented]/tabs-list:px-3 group-data-[look=segmented]/tabs-list:has-data-[icon=inline-end]:pr-2 group-data-[look=segmented]/tabs-list:has-data-[icon=inline-start]:pl-2 group-data-[look=segmented]/tabs-list:data-active:border-border group-data-[look=segmented]/tabs-list:data-active:bg-card group-data-[look=segmented]/tabs-list:data-active:text-foreground",
        className
      )}
      {...props}
    />
  )
}

function TabsContent({
  className,
  ...props
}: React.ComponentProps<typeof TabsPrimitive.Content>) {
  return (
    <TabsPrimitive.Content
      data-slot="tabs-content"
      className={cn("flex-1 rounded-lg text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background", className)}
      {...props}
    />
  )
}

export { Tabs, TabsList, TabsTrigger, TabsContent, tabsListVariants }
