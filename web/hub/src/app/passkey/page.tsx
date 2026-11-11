"use client";

import Link from "next/link";
import { useEffect, useState } from "react";
import { FingerprintIcon, KeyRoundIcon, Loader2Icon, LogOutIcon } from "lucide-react";
import { PageHeader } from "@/components/pulse/page-header";
import { Notice, Panel } from "@/components/pulse/panel";
import { useCrumbs } from "@/components/shell/crumbs";
import { Button } from "@/components/ui/button";
import { api } from "@/lib/api";
import { browserPasskeySupport, passkeySignIn, rememberSignedIn, signedIn } from "@/lib/passkey";
import {
  MEMBERS_PATH,
  membersOf,
  passkeyErrorWords,
  passkeyPageNote,
  signInWords,
  standingOf,
  type SignedIn,
} from "@/lib/passkey-state";

// /passkey/: sign in with your passkey (ADR-070 step 5). Through the front
// the brain refuses every change that is not signed by a member's passkey;
// this page proves this browser holds one and remembers which, so each
// later change asks for that passkey by name.
export default function PasskeyPage() {
  useCrumbs([{ label: "Passkey" }]);
  const [host, setHost] = useState("");
  const [support, setSupport] = useState({ ok: true, why: "" });
  const [me, setMe] = useState<SignedIn | null>(null);
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<{ tone: "ok" | "warn" | "bad"; text: string } | null>(null);

  useEffect(() => {
    // Browser-only facts, read after the static page hydrates.
    /* eslint-disable react-hooks/set-state-in-effect */
    setHost(location.hostname);
    setSupport(browserPasskeySupport());
    setMe(signedIn());
    /* eslint-enable react-hooks/set-state-in-effect */
  }, []);

  const note = host ? passkeyPageNote(host, support) : null;

  const signIn = async () => {
    setBusy(true);
    setResult(null);
    try {
      const credential = await passkeySignIn();
      const members = membersOf(await api(MEMBERS_PATH));
      const standing = standingOf(members, credential);
      const words = signInWords(standing);
      setResult(words);
      const next = standing.state === "active" ? { credential, member: standing.member, at: new Date().toISOString() } : null;
      rememberSignedIn(next);
      setMe(next);
    } catch (e) {
      setResult({ tone: "bad", text: passkeyErrorWords(e) });
    } finally {
      setBusy(false);
    }
  };

  const signOut = () => {
    rememberSignedIn(null);
    setMe(null);
    setResult(null);
  };

  return (
    <div className="mx-auto max-w-2xl">
      <PageHeader
        title="Sign in with your passkey"
        subtitle="Through the front, every change you make is signed by your passkey and checked by your team's brain."
        lead={<FingerprintIcon />}
      />
      <div className="grid gap-4">
        {note ? <Notice tone={note.tone}>{note.text}</Notice> : null}
        <Panel title="This browser" icon={KeyRoundIcon}>
          <div className="grid gap-3 text-sm leading-relaxed">
            {me ? (
              <p>
                Signed in as <strong>{me.member}</strong>. Changes you make here ask for this passkey.
              </p>
            ) : (
              <p className="text-muted-foreground">
                Not signed in. Your device asks for your fingerprint, face or PIN; the passkey never leaves it.
              </p>
            )}
            <div className="flex flex-wrap gap-2">
              <Button onClick={() => void signIn()} disabled={busy || !support.ok}>
                {busy ? <Loader2Icon className="animate-spin" /> : <FingerprintIcon />} {me ? "Sign in again" : "Sign in with passkey"}
              </Button>
              {me ? (
                <Button variant="outline" onClick={signOut}>
                  <LogOutIcon /> Forget this passkey here
                </Button>
              ) : null}
            </div>
            {result ? <Notice tone={result.tone}>{result.text}</Notice> : null}
          </div>
        </Panel>
        <p className="text-sm text-muted-foreground">
          No passkey on this device yet?{" "}
          <Link href="/passkey/register/" className="text-link underline-offset-4 hover:underline">
            Register one
          </Link>{" "}
          and give the owner the line it shows.
        </p>
      </div>
    </div>
  );
}
