"use client";

import { ThemeProvider } from "next-themes";
import { TooltipProvider } from "@/components/ui/tooltip";
import { Toaster } from "@/components/ui/sonner";
import { HeldoutHost } from "@/components/hive/heldout";
import { useEffect } from "react";
import { installPasskeySigner } from "@/lib/passkey";

const toastOffset = { bottom: "var(--hive-toast-bottom)" };

export function Providers({ children }: { children: React.ReactNode }) {
  return (
    <ThemeProvider attribute="class" defaultTheme="system" enableSystem disableTransitionOnChange>
      <TooltipProvider delayDuration={200}>
        {children}
        {/* The held-out fixtures dialog (ADR-032) lives here, not on the gate card that opens it: the card leaves the inbox before the dialog is done. */}
        <HeldoutHost />
        <PasskeySigner />
      </TooltipProvider>
      {/* The compact nav stays visible through tablet widths, not just phones. */}
      <Toaster
        position="bottom-right"
        closeButton
        richColors
        className="toaster group [--hive-toast-bottom:calc(5rem+env(safe-area-inset-bottom))] lg:[--hive-toast-bottom:2rem]"
        offset={toastOffset}
        mobileOffset={toastOffset}
      />
    </ThemeProvider>
  );
}

// Through the hosted front (ADR-070 step 5) every write is passkey-signed;
// on this machine nothing is installed and writes go out unchanged.
function PasskeySigner() {
  useEffect(() => {
    installPasskeySigner();
  }, []);
  return null;
}
