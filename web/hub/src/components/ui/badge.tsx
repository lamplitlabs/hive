import * as React from "react"
import { cva, type VariantProps } from "class-variance-authority"
import { cn } from "@/lib/utils"
import { Slot } from "radix-ui"

// Small, quiet chips. A badge names a state; it never shouts. Colour comes
// from the tone, not from borders or shadows. Corners are the control radius
// (6px); `pill` rounds them fully for a count or a dot-and-word status.
const badgeVariants = cva(
  "group/badge inline-flex h-5 w-fit shrink-0 items-center justify-center gap-1 overflow-hidden border border-transparent px-2 text-xs font-medium whitespace-nowrap transition-colors duration-150 outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background has-data-[icon=inline-end]:pr-1.5 has-data-[icon=inline-start]:pl-1.5 aria-invalid:border-destructive [&>svg]:pointer-events-none [&>svg]:size-3!",
  {
    variants: {
      variant: {
        default: "bg-primary/10 text-primary [a]:hover:bg-primary/15",
        secondary:
          "bg-secondary text-secondary-foreground [a]:hover:bg-muted",
        destructive:
          "bg-destructive/10 text-destructive [a]:hover:bg-destructive/15",
        outline:
          "border-border bg-transparent text-muted-foreground [a]:hover:bg-muted [a]:hover:text-foreground",
        ghost:
          "text-muted-foreground hover:bg-muted",
        link: "text-link underline-offset-4 hover:underline",
      },
      shape: {
        chip: "rounded-md",
        pill: "rounded-full",
      },
    },
    defaultVariants: {
      variant: "default",
      shape: "chip",
    },
  }
)

function Badge({
  className,
  variant = "default",
  shape,
  pill = false,
  asChild = false,
  ...props
}: React.ComponentProps<"span"> &
  VariantProps<typeof badgeVariants> & { asChild?: boolean; pill?: boolean }) {
  const Comp = asChild ? Slot.Root : "span"
  const resolvedShape = shape ?? (pill ? "pill" : "chip")

  return (
    <Comp
      data-slot="badge"
      data-variant={variant}
      data-shape={resolvedShape}
      className={cn(badgeVariants({ variant, shape: resolvedShape }), className)}
      {...props}
    />
  )
}

export { Badge, badgeVariants }
