"use client"

import { useTheme } from "next-themes"
import { Toaster as Sonner, type ToasterProps } from "sonner"
import { InfoIcon, TriangleAlertIcon, Loader2Icon } from "lucide-react"
import { HiveMascot } from "@/components/hive/mascots"

const Toaster = ({ ...props }: ToasterProps) => {
  const { theme = "system" } = useTheme()

  return (
    <Sonner
      theme={theme as ToasterProps["theme"]}
      className="toaster group"
      icons={{
        success: (
          <HiveMascot mood="happy" size={22} decorative />
        ),
        info: (
          <InfoIcon className="size-4" />
        ),
        warning: (
          <TriangleAlertIcon className="size-4" />
        ),
        error: (
          <HiveMascot mood="worried" size={22} decorative />
        ),
        loading: (
          <Loader2Icon className="size-4 animate-spin" />
        ),
      }}
      style={
        {
          "--normal-bg": "var(--popover)",
          "--normal-text": "var(--popover-foreground)",
          "--normal-border": "var(--border)",
          "--border-radius": "var(--radius)",
          // richColors' own light-theme words sit at 3.07 to 4.36:1 on their
          // tints; the console's status inks clear 5:1 in both themes (TASK-173).
          "--success-text": "var(--ok)",
          "--info-text": "var(--info)",
          "--warning-text": "var(--warn)",
          "--error-text": "var(--bad)",
        } as React.CSSProperties
      }
      toastOptions={{
        classNames: {
          toast: "cn-toast pop rounded-xl border bg-popover shadow-pop",
        },
      }}
      {...props}
    />
  )
}

export { Toaster }
