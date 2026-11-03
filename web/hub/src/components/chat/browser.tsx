"use client";

// The Browser tab of the canvas: the agent's dev server live in a frame,
// with an address bar, reload, open-in-a-tab and Pick - point at an element
// on the page and it goes into your message as a chip, so "make this
// teal" names exactly which this; the way the GitHub Copilot app's browser
// panel and its Pick & Polish work. The frame is another origin, so the
// hub cannot see into it; a small helper the dev page includes
// (public/pick.js, served at /pick.js) does the pointing and tells the hub
// over postMessage (lib/pick.ts is the contract). Without the helper the
// tab still shows the page and offers to have the agent add it.

import { useEffect, useRef, useState } from "react";
import {
  CopyIcon,
  ExternalLinkIcon,
  GlobeIcon,
  LoaderCircleIcon,
  RotateCwIcon,
  SquareDashedMousePointerIcon,
  WandSparklesIcon,
  XIcon,
} from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Notice } from "@/components/pulse/panel";
import { browserAddress, isLocalUrl } from "@/lib/canvas";
import {
  askForHelper,
  pickLabel,
  pickPlace,
  pickRequest,
  pickSnippet,
  readPickMessage,
  type ElementPick,
} from "@/lib/pick";
import { cn } from "@/lib/utils";

/** What the composer offers the Browser tab: the picks it holds. */
export interface PickSink {
  picks: ElementPick[];
  add: (p: ElementPick) => void;
  remove: (key: string) => void;
}

// The frame's host is the one from `url`; the owner may type another
// local address into the bar. A web address is refused (a site rarely
// lets itself be framed, and the hub's policy does not allow it).
export function BrowserTab({
  url,
  sink,
  onNavigate,
  onAsk,
}: {
  url: string;
  sink?: PickSink;
  /** The address the owner typed, so the tab can be re-keyed. */
  onNavigate?: (url: string) => void;
  /** Puts words into the composer (the ask for the helper). */
  onAsk?: (text: string) => void;
}) {
  const [address, setAddress] = useState(url);
  const [src, setSrc] = useState(url ? browserAddress(url) : "");
  const [nonce, setNonce] = useState(0);
  const [loading, setLoading] = useState(!!url);
  const [picking, setPicking] = useState(false);
  // "unknown" until the helper says hello; it only can once the page has
  // the script and the hub has spoken to it (the frame's first load).
  const [helper, setHelper] = useState<"unknown" | "yes">("unknown");
  const [pageUrl, setPageUrl] = useState("");
  const frame = useRef<HTMLIFrameElement | null>(null);

  // A new address from outside (the tab re-keyed) starts the frame over.
  // Derived during render, the way useCanvas tracks its session.
  const [seenUrl, setSeenUrl] = useState(url);
  if (url !== seenUrl) {
    setSeenUrl(url);
    setAddress(url);
    setSrc(url ? browserAddress(url) : "");
    setHelper("unknown");
    setPicking(false);
    setLoading(!!url);
  }

  const target = () => {
    try {
      return new URL(src).origin;
    } catch {
      return "";
    }
  };

  // The helper speaks only to the window that framed it, from the frame
  // we hold; anything else the window hears is not for us.
  useEffect(() => {
    const onMessage = (e: MessageEvent) => {
      if (!frame.current || e.source !== frame.current.contentWindow) return;
      const m = readPickMessage(e.data);
      if (!m) return;
      if (m.type === "hello") {
        setHelper("yes");
        if (m.url) setPageUrl(m.url);
        return;
      }
      if (m.type === "cancel") {
        setPicking(false);
        return;
      }
      setPicking(false);
      if (sink) {
        sink.add(m.pick);
        toast.success("Picked " + pickLabel(m.pick) + ". Say what to change.");
      }
    };
    window.addEventListener("message", onMessage);
    return () => window.removeEventListener("message", onMessage);
  }, [sink]);

  const tell = (on: boolean) => {
    const w = frame.current?.contentWindow;
    const origin = target();
    if (!w || !origin) return;
    w.postMessage(pickRequest(on), origin);
  };

  // On load, greet the helper so it learns the hub's origin and answers
  // with hello; a page without the helper stays silent.
  const loaded = () => {
    setLoading(false);
    tell(false);
  };

  const togglePick = () => {
    const next = !picking;
    setPicking(next);
    tell(next);
  };

  const go = (raw: string) => {
    let next = raw.trim();
    if (!next) return;
    if (!/^https?:\/\//i.test(next)) next = "http://" + next;
    if (!isLocalUrl(next)) {
      toast.error(
        "The Browser tab shows pages served from this machine (localhost, 127.0.0.1). Open other sites in a new tab.",
      );
      return;
    }
    setAddress(next);
    setSrc(browserAddress(next));
    setLoading(true);
    setHelper("unknown");
    setPicking(false);
    onNavigate?.(next);
  };

  const reload = () => {
    setLoading(true);
    setHelper("unknown");
    setPicking(false);
    setNonce((n) => n + 1);
  };

  const hubOrigin =
    typeof window !== "undefined" ? window.location.origin : "";
  const snippet = pickSnippet(hubOrigin);
  const picks = sink?.picks ?? [];
  const blank = !src;

  return (
    <>
      <form
        className="flex min-w-0 items-center gap-1.5 border-b px-2 py-1.5"
        onSubmit={(e) => {
          e.preventDefault();
          go(address);
        }}
      >
        <Button
          type="button"
          size="icon-xs"
          variant="ghost"
          onClick={reload}
          aria-label="Reload the page"
          title="Reload"
          className="shrink-0"
        >
          {loading ? (
            <LoaderCircleIcon className="animate-spin" />
          ) : (
            <RotateCwIcon />
          )}
        </Button>
        <Input
          value={address}
          onChange={(e) => setAddress(e.target.value)}
          onBlur={() => setAddress(pageUrl && helper === "yes" ? pageUrl : address)}
          aria-label="Address"
          placeholder="http://localhost:3000/"
          spellCheck={false}
          autoComplete="off"
          autoFocus={blank}
          className="h-7 min-w-0 flex-1 rounded-md bg-background px-2 font-mono text-xs"
        />
        {sink && !blank ? (
          <Button
            type="button"
            size="xs"
            variant={picking ? "default" : "outline"}
            onClick={togglePick}
            disabled={loading && !picking}
            aria-pressed={picking}
            aria-label={picking ? "Stop picking" : "Pick an element on the page"}
            title={
              helper === "yes"
                ? "Pick an element: hover and click one on the page, Esc to stop."
                : "Pick needs the PULSE helper on the page; press for how."
            }
            className="shrink-0"
          >
            <SquareDashedMousePointerIcon />
            {picking ? "Picking…" : "Pick"}
          </Button>
        ) : null}
        {blank ? null : (
          <Button asChild size="icon-xs" variant="ghost" className="shrink-0">
            <a
              href={src}
              target="_blank"
              rel="noreferrer noopener"
              aria-label="Open in a new tab"
              title="Open in a new tab"
            >
              <ExternalLinkIcon />
            </a>
          </Button>
        )}
      </form>
      {picking && helper !== "yes" ? (
        <div className="border-b px-3 py-2">
          <Notice tone="info" className="py-2 text-[12.5px]">
            <span className="grid min-w-0 gap-1.5">
              <span>
                This page has no PULSE pick helper yet, so the hub cannot see
                what you point at. Add one line to the app&apos;s development
                pages:
              </span>
              <code className="block min-w-0 rounded-md bg-muted/60 px-2 py-1 font-mono text-[11.5px] anywhere">
                {snippet}
              </code>
              <span className="flex min-w-0 flex-wrap items-center gap-1.5">
                <Button
                  type="button"
                  size="xs"
                  variant="outline"
                  onClick={() =>
                    navigator.clipboard
                      .writeText(snippet)
                      .then(() => toast.success("Copied."))
                      .catch(() =>
                        toast.error("Couldn't copy. Select it and copy it yourself."),
                      )
                  }
                >
                  <CopyIcon /> Copy
                </Button>
                {onAsk ? (
                  <Button
                    type="button"
                    size="xs"
                    variant="outline"
                    onClick={() => {
                      onAsk(askForHelper(hubOrigin, src));
                      setPicking(false);
                    }}
                  >
                    <WandSparklesIcon /> Ask the agent to add it
                  </Button>
                ) : null}
                <span className="text-muted-foreground">
                  Then reload. If you added it already, give the page a moment
                  and press Pick again.
                </span>
              </span>
            </span>
          </Notice>
        </div>
      ) : null}
      {picks.length && sink ? (
        <ul
          className="flex min-w-0 flex-wrap items-center gap-1.5 border-b bg-primary/[0.04] px-3 py-1.5"
          aria-label="Picked elements"
        >
          {picks.map((p) => (
            <li
              key={p.key}
              className="pop flex min-w-0 max-w-72 items-center gap-1.5 rounded-lg border border-primary/30 bg-card py-0.5 pr-0.5 pl-2 text-xs"
              title={p.selector + "\n" + p.url}
            >
              <SquareDashedMousePointerIcon
                className="size-3.5 shrink-0 text-primary"
                aria-hidden
              />
              <span className="min-w-0 truncate font-mono text-foreground">
                {pickLabel(p)}
              </span>
              <span className="hidden shrink-0 text-muted-foreground sm:inline">
                {pickPlace(p.url)}
              </span>
              <button
                type="button"
                onClick={() => sink.remove(p.key)}
                className="rounded-md p-0.5 text-muted-foreground hover:bg-muted hover:text-foreground"
                aria-label={"Remove " + pickLabel(p)}
              >
                <XIcon className="size-3" />
              </button>
            </li>
          ))}
          <li className="text-[11px] text-muted-foreground">
            In your next message.
          </li>
        </ul>
      ) : null}
      {blank ? (
        <div className="grid min-h-[20rem] flex-1 content-center justify-items-center gap-2 px-6 py-8 text-center">
          <GlobeIcon
            className="size-[18px] text-muted-foreground"
            aria-hidden
          />
          <p className="max-w-sm text-sm leading-relaxed text-muted-foreground">
            Type the address of a page served on this machine - the
            agent&apos;s dev server, a preview build - and press Enter. A
            server the agent starts opens here by itself.
          </p>
        </div>
      ) : (
        <div
          className={cn(
            "relative min-h-[20rem] min-w-0 flex-1 bg-white",
            "max-h-[70dvh] lg:max-h-none",
          )}
        >
          <iframe
            key={src + "#" + nonce}
            ref={frame}
            src={src}
            title={"Browser: " + address}
            onLoad={loaded}
            // The page is the agent's own work on this machine: it may run
            // scripts, keep its own origin (so its fetches and storage work),
            // open forms and pop a window out. It cannot reach the hub's page
            // or storage: the hub is another origin.
            sandbox="allow-scripts allow-same-origin allow-forms allow-popups allow-modals allow-downloads"
            referrerPolicy="no-referrer"
            className={cn(
              "block h-full min-h-[20rem] w-full",
              picking && "ring-2 ring-primary ring-inset",
            )}
          />
          {picking ? (
            <div
              className="pointer-events-none absolute inset-x-0 top-0 flex justify-center pt-2"
              aria-live="polite"
            >
              <span className="pop rounded-full border border-primary/40 bg-card/95 px-3 py-1 text-xs font-medium text-foreground shadow-pop">
                Click an element on the page. Esc stops.
              </span>
            </div>
          ) : null}
        </div>
      )}
    </>
  );
}
