import * as React from "react"
import { cva, type VariantProps } from "class-variance-authority"
import { cn } from "@/lib/utils"
import { Slot } from "radix-ui"

// Flat, solid buttons. The primary is ink (the logo's navy, white text; a
// near-white with dark text in the dark theme), used for the one action a
// view is for; outline is the everyday button; ghost sits in toolbars;
// destructive is for declines and removals; link is teal, like every link.
// No gradients, no lift on hover - a button says what it does, darkens when
// pressed and squishes a little under the finger. Corners are the control
// radius (6px); heights are 36px default, 32px small, 28px xs, 40px lg.
const buttonVariants = cva(
  "group/button inline-flex shrink-0 items-center justify-center rounded-md border border-transparent bg-clip-padding text-sm font-medium whitespace-nowrap transition-[color,background-color,border-color,box-shadow,transform] duration-150 ease-out-soft outline-none select-none active:scale-[0.96] motion-reduce:active:scale-100 focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background disabled:pointer-events-none disabled:opacity-50 aria-invalid:border-destructive aria-invalid:ring-3 aria-invalid:ring-destructive/20 aria-invalid:focus-visible:ring-destructive [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-4",
  {
    variants: {
      variant: {
        default:
          "bg-primary text-primary-foreground hover:bg-primary-strong active:bg-primary-strong",
        outline:
          "border-input bg-card text-foreground hover:border-[color-mix(in_oklch,var(--foreground)_22%,var(--input))] hover:bg-muted/60 aria-expanded:bg-muted dark:bg-card dark:hover:bg-muted",
        secondary:
          "bg-secondary text-secondary-foreground hover:bg-[color-mix(in_oklch,var(--secondary),var(--foreground)_6%)] aria-expanded:bg-[color-mix(in_oklch,var(--secondary),var(--foreground)_6%)]",
        ghost:
          "text-foreground hover:bg-muted aria-expanded:bg-muted",
        destructive:
          "bg-destructive text-white hover:bg-[color-mix(in_oklch,var(--destructive),black_12%)] focus-visible:ring-destructive dark:text-[oklch(0.18_0.03_25)] dark:hover:bg-[color-mix(in_oklch,var(--destructive),white_10%)]",
        link: "h-auto! px-0! text-link underline-offset-4 hover:underline",
      },
      size: {
        default:
          "h-9 gap-2 px-3.5 text-sm font-medium has-data-[icon=inline-end]:pr-3 has-data-[icon=inline-start]:pl-3",
        xs: "h-7 gap-1 px-2 text-xs pointer-coarse:h-9 has-data-[icon=inline-end]:pr-1.5 has-data-[icon=inline-start]:pl-1.5 [&_svg:not([class*='size-'])]:size-3.5",
        sm: "h-8 gap-1.5 px-3 text-[13px] pointer-coarse:h-9 has-data-[icon=inline-end]:pr-2 has-data-[icon=inline-start]:pl-2 [&_svg:not([class*='size-'])]:size-3.5",
        lg: "h-10 gap-2 px-4 text-[15px] has-data-[icon=inline-end]:pr-3.5 has-data-[icon=inline-start]:pl-3.5",
        icon: "size-9",
        "icon-xs":
          "size-7 pointer-coarse:size-10 [&_svg:not([class*='size-'])]:size-3.5",
        "icon-sm":
          "size-8 pointer-coarse:size-10",
        "icon-lg": "size-10",
      },
    },
    defaultVariants: {
      variant: "default",
      size: "default",
    },
  }
)

function Button({
  className,
  variant = "default",
  size = "default",
  asChild = false,
  ...props
}: React.ComponentProps<"button"> &
  VariantProps<typeof buttonVariants> & {
    asChild?: boolean
  }) {
  const Comp = asChild ? Slot.Root : "button"

  return (
    <Comp
      data-slot="button"
      data-variant={variant}
      data-size={size}
      className={cn(buttonVariants({ variant, size, className }))}
      {...props}
    />
  )
}

export { Button, buttonVariants }
