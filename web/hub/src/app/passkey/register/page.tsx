"use client";

import Link from "next/link";
import { useEffect, useState } from "react";
import { FingerprintIcon, KeyRoundIcon, Loader2Icon } from "lucide-react";
import { CommandBox } from "@/components/pulse/evolve";
import { PageHeader } from "@/components/pulse/page-header";
import { Notice, Panel } from "@/components/pulse/panel";
import { useCrumbs } from "@/components/shell/crumbs";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { browserPasskeySupport, registerPasskey } from "@/lib/passkey";
import { memberNameProblem, passkeyAddCommand, passkeyErrorWords, passkeyPageNote, type Registration } from "@/lib/passkey-state";

// /passkey/register/: make a passkey for this team's front on this device
// (ADR-070 step 5). The brain trusts only public keys the owner adds, so the
// page ends with the one line the owner runs on the brain:
// `pulse member passkey add MEMBER --credential ID --public-key KEY`.
export default function RegisterPasskeyPage() {
  useCrumbs([{ label: "Passkey", href: "/passkey/" }, { label: "Register" }]);
  const [host, setHost] = useState("");
  const [support, setSupport] = useState({ ok: true, why: "" });
  const [member, setMember] = useState("");
  const [touched, setTouched] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [made, setMade] = useState<Registration | null>(null);

  useEffect(() => {
    /* eslint-disable react-hooks/set-state-in-effect */
    setHost(location.hostname);
    setSupport(browserPasskeySupport());
    /* eslint-enable react-hooks/set-state-in-effect */
  }, []);

  const note = host ? passkeyPageNote(host, support) : null;
  const problem = memberNameProblem(member);

  const register = async () => {
    setTouched(true);
    if (problem) return;
    setBusy(true);
    setError("");
    setMade(null);
    try {
      setMade(await registerPasskey(member.trim()));
    } catch (e) {
      setError(passkeyErrorWords(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="mx-auto max-w-2xl">
      <PageHeader
        title="Register a passkey"
        subtitle="Make a passkey on this device for your team's console. The owner then adds it on the brain."
        lead={<KeyRoundIcon />}
      />
      <div className="grid gap-4">
        {note ? <Notice tone={note.tone}>{note.text}</Notice> : null}
        <Panel title="1. Make the passkey" icon={FingerprintIcon}>
          <form
            className="grid gap-3"
            onSubmit={(e) => {
              e.preventDefault();
              void register();
            }}
          >
            <div className="grid gap-1.5">
              <Label htmlFor="passkey-member">Your member name</Label>
              <Input
                id="passkey-member"
                value={member}
                onChange={(e) => setMember(e.target.value)}
                onBlur={() => setTouched(true)}
                placeholder="ana@acme.dev"
                autoComplete="username webauthn"
                spellCheck={false}
                aria-invalid={touched && !!problem}
                aria-describedby="passkey-member-help"
              />
              <p id="passkey-member-help" className={touched && problem ? "text-xs text-bad" : "text-xs text-muted-foreground"}>
                {touched && problem ? problem : "The name the owner gave you with pulse member add."}
              </p>
            </div>
            <div>
              <Button type="submit" disabled={busy || !support.ok}>
                {busy ? <Loader2Icon className="animate-spin" /> : <FingerprintIcon />} Make passkey
              </Button>
            </div>
            {error ? <Notice tone="bad">{error}</Notice> : null}
          </form>
        </Panel>
        {made ? (
          <Panel title="2. Send this to the owner" icon={KeyRoundIcon}>
            <div className="grid gap-3 text-sm leading-relaxed">
              <p>
                The passkey is on this device. It holds no secret you need to hide - only its public half is below. The owner runs this on the brain:
              </p>
              <CommandBox command={passkeyAddCommand(made)} copy />
              <p className="text-muted-foreground">
                Once it is added,{" "}
                <Link href="/passkey/" className="text-link underline-offset-4 hover:underline">
                  sign in with it
                </Link>
                .
              </p>
            </div>
          </Panel>
        ) : null}
      </div>
    </div>
  );
}
