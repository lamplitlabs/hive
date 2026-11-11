"use client";

import { useRef } from "react";
import { CheckIcon } from "lucide-react";
import { Dialog as DialogPrimitive } from "radix-ui";
import { Dialog, DialogContent, DialogDescription, DialogPortal, DialogTitle } from "@/components/ui/dialog";
import { cn } from "@/lib/utils";

// The parts every "add" wizard shares (a machine, a tool, a skill, a model
// provider): numbered steps across the top, a title with one line under
// it, a body that scrolls, and a footer that stays put with Back on the
// left and the next step on the right. Unlike the console's other panels a
// wizard is modal: one focused task over a dimmed page, so a half-done
// setup isn't lost behind a stray click.

export interface WizardStep {
  id: string;
  label: string;
}

// The numbered steps. Steps already reached are buttons back to them; a
// finished wizard (`done`) ticks every step and locks them.
export function Stepper({ steps, current, reach, done, onGo }: { steps: WizardStep[]; current: string; reach: number; done?: boolean; onGo: (id: string) => void }) {
  const at = steps.findIndex((s) => s.id === current);
  return (
    <ol className="flex min-w-0 items-center gap-1.5 sm:gap-2" aria-label="Steps">
      {steps.map((s, i) => {
        const past = done || i < at;
        const here = !done && i === at;
        const can = !done && i <= reach && !here;
        return (
          <li key={s.id} className="flex min-w-0 items-center gap-1.5 sm:gap-2">
            {i ? <span aria-hidden className={cn("h-px w-3 shrink-0 sm:w-6", past || here ? "bg-primary/60" : "bg-border")} /> : null}
            <button
              type="button"
              disabled={!can}
              onClick={() => onGo(s.id)}
              aria-current={here ? "step" : undefined}
              className={cn(
                "flex min-w-0 items-center gap-1.5 rounded-full py-0.5 pr-2 pl-0.5 text-[13px] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
                here ? "font-semibold text-foreground" : past ? "text-foreground" : "text-muted-foreground",
                can && "hover:bg-muted",
              )}
            >
              <span
                aria-hidden
                className={cn(
                  "inline-flex size-5 shrink-0 items-center justify-center rounded-full text-xs font-semibold tnum",
                  past ? "bg-primary text-primary-foreground" : here ? "bg-primary/12 text-primary ring-1 ring-primary/40" : "bg-muted text-muted-foreground",
                )}
              >
                {past ? <CheckIcon className="size-3" /> : i + 1}
              </span>
              <span className={cn("truncate", !here && "hidden sm:inline")}>{s.label}</span>
            </button>
          </li>
        );
      })}
    </ol>
  );
}

// The modal frame. The body is rendered only while open, so every opening
// starts fresh. To keep it open while a save is in flight, the caller's
// onOpenChange ignores a close (see useWizardLock).
export function WizardDialog({ open, onOpenChange, children, className }: { open: boolean; onOpenChange: (open: boolean) => void; children: React.ReactNode; className?: string }) {
  return (
    <Dialog modal open={open} onOpenChange={onOpenChange}>
      <DialogPortal>
        <DialogPrimitive.Overlay className="fixed inset-0 z-50 bg-[oklch(0.22_0.03_262/.32)] data-open:animate-in data-open:fade-in-0 data-closed:animate-out data-closed:fade-out-0" />
      </DialogPortal>
      <DialogContent placement="float" className={cn("flex max-h-[min(86dvh,760px)] flex-col gap-0 overflow-hidden p-0 sm:max-w-2xl", className)}>
        {open ? children : null}
      </DialogContent>
    </Dialog>
  );
}

// The ref a wizard body sets while it saves, so a close request waits.
export function useWizardLock() {
  return useRef(false);
}

export function WizardHeader({ steps, current, reach, done, onGo, title, description }: { steps: WizardStep[]; current: string; reach: number; done?: boolean; onGo: (id: string) => void; title: React.ReactNode; description?: React.ReactNode }) {
  return (
    <div className="grid gap-3 border-b px-5 pt-5 pb-4">
      <Stepper steps={steps} current={current} reach={reach} done={done} onGo={onGo} />
      <div className="pr-8">
        <DialogTitle className="text-lg font-semibold leading-snug">{title}</DialogTitle>
        {description ? <DialogDescription className="mt-0.5 text-[13px] leading-relaxed text-muted-foreground anywhere">{description}</DialogDescription> : <DialogDescription className="sr-only">{typeof title === "string" ? title : "Wizard"}</DialogDescription>}
      </div>
    </div>
  );
}

export function WizardBody({ children, className, bodyRef }: { children: React.ReactNode; className?: string; bodyRef?: React.Ref<HTMLDivElement> }) {
  return (
    <div ref={bodyRef} className={cn("scroll-thin min-h-0 flex-1 overflow-y-auto px-5 py-5", className)}>
      {children}
    </div>
  );
}

// Back (or a quiet note) on the left, the next step on the right; `why`
// says what is missing while the next step is disabled.
export function WizardFooter({ left, right, why }: { left?: React.ReactNode; right: React.ReactNode; why?: string }) {
  return (
    <div className="flex items-center justify-between gap-2 border-t bg-muted/30 px-5 py-3">
      <div className="min-w-0">{left}</div>
      <div className="flex min-w-0 items-center gap-3">
        {why ? <span className="hidden text-right text-[13px] leading-snug text-muted-foreground sm:block">{why}</span> : null}
        {right}
      </div>
    </div>
  );
}

// Big choice cards for a wizard's first step ("Where is it?").
export function ChoiceCard({ selected, onSelect, icon: Icon, title, children, badge, disabled }: { selected: boolean; onSelect: () => void; icon: React.ComponentType<{ className?: string }>; title: string; children: React.ReactNode; badge?: React.ReactNode; disabled?: boolean }) {
  return (
    <button
      type="button"
      role="radio"
      aria-checked={selected}
      disabled={disabled}
      onClick={onSelect}
      className={cn(
        "grid min-w-0 gap-1.5 rounded-xl border bg-card p-3.5 text-left transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-60",
        selected ? "border-primary ring-1 ring-primary" : "hover:border-foreground/25 hover:bg-muted/40",
      )}
    >
      <span className="flex min-w-0 items-center gap-2.5">
        <span aria-hidden className={cn("inline-flex size-8 shrink-0 items-center justify-center rounded-lg", selected ? "bg-primary/12 text-primary" : "bg-muted text-muted-foreground")}>
          <Icon className="size-4" />
        </span>
        <span className="min-w-0 flex-1 truncate text-sm font-semibold">{title}</span>
        {badge}
        <span aria-hidden className={cn("inline-flex size-4 shrink-0 items-center justify-center rounded-full border", selected ? "border-primary bg-primary text-primary-foreground" : "border-input")}>
          {selected ? <CheckIcon className="size-3" /> : null}
        </span>
      </span>
      <span className="text-[13px] leading-snug text-muted-foreground">{children}</span>
    </button>
  );
}
