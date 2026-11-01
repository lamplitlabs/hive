"use client";

// The microphone in the composer: hold the model on this machine, press
// to record, press again to transcribe; the words land where the caret
// is. The first press sets the speech model up (NVIDIA Nemotron through
// ONNX Runtime, the model GitHub Copilot's CLI dictates with) - a 2.5 GB
// download the dialog here shows and can stop. Nothing recorded leaves
// the computer: the hub transcribes in its own process.

import { useCallback, useEffect, useRef, useState } from "react";
import {
  LoaderCircleIcon,
  MicIcon,
  MicOffIcon,
  SquareIcon,
  Trash2Icon,
} from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Progress } from "@/components/ui/progress";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { errorText, voiceTranscribe, writes } from "@/lib/api";
import { invalidate, load, peek, useApi } from "@/lib/use-api";
import { cn } from "@/lib/utils";
import {
  VOICE_SAMPLE_RATE,
  gigs,
  micState,
  micTitle,
  pcm16,
  recordingProblem,
  resample,
  setupLine,
  setupPercent,
  type Transcript,
  type VoiceStatus,
} from "@/lib/voice";

const VOICE_PATH = "voice";

// The recorder: the microphone through an AudioContext, mono float frames
// gathered until stop, then 16 kHz PCM. A ScriptProcessor is used because
// it needs no worklet file to ship with the export; the deprecation is a
// warning, and every browser the hub runs in still has it.
class Recorder {
  private chunks: Float32Array[] = [];
  private ctx: AudioContext | null = null;
  private stream: MediaStream | null = null;
  private node: ScriptProcessorNode | null = null;
  private source: MediaStreamAudioSourceNode | null = null;
  rate = VOICE_SAMPLE_RATE;

  async start(): Promise<void> {
    if (!navigator.mediaDevices?.getUserMedia) {
      throw new Error(
        "This browser gives pages no microphone. Open the hub on 127.0.0.1 or over https.",
      );
    }
    // Plain `audio: true`: the browser's default microphone with its own
    // echo cancellation and gain, which is what dictation wants, and the
    // request every device answers (constraint sets stall some). The wait
    // is bounded so a prompt left open does not leave the button stuck.
    const asked = navigator.mediaDevices.getUserMedia({ audio: true });
    this.stream = await Promise.race([
      asked,
      new Promise<MediaStream>((_, reject) =>
        setTimeout(
          () =>
            reject(
              new Error(
                "The browser did not open the microphone within 30 seconds. Allow it for this site and try again.",
              ),
            ),
          30_000,
        ),
      ),
    ]).catch((e) => {
      // A late answer must not leave a live track behind.
      void asked.then((s) => s.getTracks().forEach((t) => t.stop())).catch(() => {});
      throw e;
    });
    let ctx: AudioContext;
    try {
      ctx = new AudioContext({ sampleRate: VOICE_SAMPLE_RATE });
    } catch {
      ctx = new AudioContext();
    }
    this.ctx = ctx;
    this.rate = ctx.sampleRate;
    this.source = ctx.createMediaStreamSource(this.stream);
    this.node = ctx.createScriptProcessor(4096, 1, 1);
    this.node.onaudioprocess = (e) => {
      this.chunks.push(new Float32Array(e.inputBuffer.getChannelData(0)));
    };
    this.source.connect(this.node);
    // A processor with no destination never runs in some browsers; a muted
    // gain keeps the graph alive without playing the microphone back.
    const mute = ctx.createGain();
    mute.gain.value = 0;
    this.node.connect(mute);
    mute.connect(ctx.destination);
    if (ctx.state === "suspended") await ctx.resume();
  }

  // Seconds captured so far.
  seconds(): number {
    const n = this.chunks.reduce((a, c) => a + c.length, 0);
    return n / this.rate;
  }

  async stop(): Promise<Float32Array> {
    this.node?.disconnect();
    this.source?.disconnect();
    this.stream?.getTracks().forEach((t) => t.stop());
    const ctx = this.ctx;
    this.ctx = null;
    this.node = null;
    this.source = null;
    this.stream = null;
    if (ctx && ctx.state !== "closed") await ctx.close().catch(() => {});
    const n = this.chunks.reduce((a, c) => a + c.length, 0);
    const all = new Float32Array(n);
    let at = 0;
    for (const c of this.chunks) {
      all.set(c, at);
      at += c.length;
    }
    this.chunks = [];
    return resample(all, this.rate, VOICE_SAMPLE_RATE);
  }
}

export function MicButton({
  disabled,
  onText,
  className,
}: {
  disabled: boolean;
  // The words, to put where the caret is.
  onText: (t: Transcript) => void;
  className?: string;
}) {
  const [recording, setRecording] = useState(false);
  const [transcribing, setTranscribing] = useState(false);
  const [elapsed, setElapsed] = useState(0);
  const [dialog, setDialog] = useState(false);
  const recorder = useRef<Recorder | null>(null);
  // The status is read once the button is on the page, then only while
  // a setup runs (every 2 s) or the dialog is open. `peek` sees the last
  // answer before this render's hook does, so the first poll rate is right.
  const watch = !!peek<VoiceStatus>(VOICE_PATH)?.setting_up || dialog;
  const status = useApi<VoiceStatus>(VOICE_PATH, {
    refresh: watch ? 2000 : 60000,
  });
  const st = status.data;
  const state = micState(st, recording, transcribing);
  // A setup that finished while the dialog was closed deserves a word.
  const lastPhase = useRef("");
  useEffect(() => {
    const phase = st?.setup?.phase ?? "";
    if (lastPhase.current && lastPhase.current !== phase && !dialog) {
      if (phase === "done")
        toast.success("The speech model is ready", {
          description: "Press the microphone to dictate.",
        });
      else if (phase === "failed")
        toast.error("The speech model setup failed", {
          description: st?.setup?.error ?? "",
        });
    }
    lastPhase.current = phase;
  }, [st?.setup?.phase, st?.setup?.error, dialog]);

  const maxSeconds = st?.max_seconds ?? 300;
  // The recording clock, which also stops the take at the route's limit.
  const stopRef = useRef<() => Promise<void>>(async () => {});
  useEffect(() => {
    if (!recording) return;
    const t = setInterval(() => {
      const s = recorder.current?.seconds() ?? 0;
      setElapsed(s);
      if (s >= maxSeconds) void stopRef.current();
    }, 250);
    return () => clearInterval(t);
  }, [recording, maxSeconds]);

  const stopAndTranscribe = useCallback(async () => {
    const r = recorder.current;
    recorder.current = null;
    setRecording(false);
    if (!r) return;
    let samples: Float32Array;
    try {
      samples = await r.stop();
    } catch (e) {
      toast.error("The recording could not be read", {
        description: String((e as Error)?.message ?? e),
      });
      return;
    }
    const seconds = samples.length / VOICE_SAMPLE_RATE;
    const problem = recordingProblem(seconds, maxSeconds);
    if (problem) {
      toast.error("Nothing to transcribe", { description: problem });
      return;
    }
    setTranscribing(true);
    try {
      const t = await voiceTranscribe(pcm16(samples));
      if (t.text) onText(t);
      else
        toast.message("No words heard", {
          description: "The model found no speech in " + seconds.toFixed(1) + " seconds of audio.",
        });
    } catch (e) {
      const t = errorText(e);
      toast.error("Transcription failed", {
        description: [t.why, t.fix].filter(Boolean).join(" ") || t.error,
      });
      // A refusal because the files are gone: re-read the status so the
      // button turns back into the setup.
      void load(VOICE_PATH, true);
    } finally {
      setTranscribing(false);
    }
  }, [maxSeconds, onText]);
  useEffect(() => {
    stopRef.current = stopAndTranscribe;
  }, [stopAndTranscribe]);

  const press = async () => {
    if (disabled || transcribing) return;
    if (recording) {
      await stopAndTranscribe();
      return;
    }
    if (!st) {
      // First press before the status arrived: read it now, and open the
      // setup unless everything is already in place.
      await load(VOICE_PATH, true);
    }
    const cur = st ?? peek<VoiceStatus>(VOICE_PATH);
    if (!cur || !cur.supported || !cur.ready || cur.setting_up) {
      setDialog(true);
      return;
    }
    const r = new Recorder();
    try {
      await r.start();
    } catch (e) {
      const msg = String((e as Error)?.message ?? e);
      toast.error("No microphone", {
        description: /denied|permission|NotAllowed/i.test(msg)
          ? "The browser did not allow the microphone. Allow it for this site and try again."
          : msg,
      });
      return;
    }
    recorder.current = r;
    setElapsed(0);
    setRecording(true);
  };

  // Escape while recording drops the take.
  useEffect(() => {
    if (!recording) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      const r = recorder.current;
      recorder.current = null;
      setRecording(false);
      void r?.stop();
      toast.message("Recording dropped");
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [recording]);

  useEffect(
    () => () => {
      void recorder.current?.stop();
    },
    [],
  );

  const Icon =
    state === "recording"
      ? SquareIcon
      : state === "transcribing" || state === "fetching"
        ? LoaderCircleIcon
        : state === "unsupported"
          ? MicOffIcon
          : MicIcon;
  return (
    <>
      <Button
        type="button"
        size="icon-sm"
        variant={state === "recording" ? "destructive" : "ghost"}
        onClick={() => void press()}
        disabled={disabled || state === "transcribing"}
        aria-label={
          state === "recording"
            ? "Stop recording and transcribe"
            : state === "fetching"
              ? "Speech model setup in progress"
              : "Dictate"
        }
        aria-pressed={state === "recording"}
        title={micTitle(state, st)}
        className={cn(
          "shrink-0 rounded-full text-muted-foreground hover:text-foreground",
          state === "recording" && "text-destructive-foreground hover:text-destructive-foreground",
          className,
        )}
      >
        <Icon
          className={cn(
            (state === "transcribing" || state === "fetching") && "animate-spin",
            state === "recording" && "fill-current",
          )}
        />
      </Button>
      {state === "recording" ? (
        <span
          className="flex items-center gap-1.5 text-xs text-destructive tnum"
          aria-live="polite"
        >
          <span className="size-2 animate-pulse rounded-full bg-destructive" aria-hidden />
          {Math.floor(elapsed / 60)}:{String(Math.floor(elapsed % 60)).padStart(2, "0")}
        </span>
      ) : null}
      <VoiceSetupDialog open={dialog} onOpenChange={setDialog} status={st} />
    </>
  );
}

// The setup: which model, how big, how far the download is, stop and
// remove. Opens from a press when the files are missing, and from the
// tooltip's gear otherwise.
export function VoiceSetupDialog({
  open,
  onOpenChange,
  status,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  status: VoiceStatus | undefined;
}) {
  const [busy, setBusy] = useState("");
  const [picked, setPicked] = useState("");
  const model = picked || status?.model.id || "";
  const chosen = status?.models.find((m) => m.id === model) ?? status?.model;
  const setup = status?.setup ?? null;
  const running = !!status?.setting_up;
  const act = async (what: string, f: () => Promise<unknown>) => {
    setBusy(what);
    try {
      await f();
      invalidate(VOICE_PATH);
      await load(VOICE_PATH, true);
    } catch (e) {
      const t = errorText(e);
      toast.error("Voice setup", {
        description: [t.why, t.fix].filter(Boolean).join(" ") || t.error,
      });
    } finally {
      setBusy("");
    }
  };
  const need = chosen ? chosen.size - chosen.have + (status?.runtime && !status.runtime.ready ? status.runtime.size : 0) : 0;
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>Dictation</DialogTitle>
          <DialogDescription>
            The hub transcribes on this machine with NVIDIA&apos;s Nemotron
            speech model through ONNX Runtime - the model GitHub Copilot
            dictates with. Nothing you say leaves the computer. The files are
            fetched once into <code className="font-mono">{status?.root ?? "PULSE_HOME/voice"}</code>.
          </DialogDescription>
        </DialogHeader>
        {!status ? (
          <p className="text-sm text-muted-foreground">Reading the status…</p>
        ) : !status.supported ? (
          <p className="text-sm">
            There is no speech runtime for this machine ({status.platform || "unknown platform"}).
            Dictation needs an Apple-silicon Mac or an x86-64/arm64 Linux machine.
          </p>
        ) : (
          <div className="grid gap-3 text-sm">
            <label className="grid gap-1">
              <span className="text-xs font-medium text-muted-foreground">Model</span>
              <Select
                value={model}
                onValueChange={setPicked}
                disabled={running || !!busy}
              >
                <SelectTrigger aria-label="Speech model" className="w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {status.models.map((m) => (
                    <SelectItem key={m.id} value={m.id}>
                      <span className="grid">
                        <span>{m.title}</span>
                        <span className="text-xs text-muted-foreground">
                          {m.languages} · {gigs(m.size)}
                          {m.ready ? " · on this machine" : m.have ? " · " + gigs(m.have) + " fetched" : ""}
                        </span>
                      </span>
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </label>
            {chosen?.ready && status.runtime?.ready ? (
              <p>
                <span className="font-medium">{chosen.title}</span> is set up
                {status.loaded ? " and in memory" : ""}. Press the microphone to dictate; the model is
                let go after {status.idle_release_minutes} minutes of silence.
                {status.language && status.language !== "auto" ? " Language: " + status.language + "." : ""}
              </p>
            ) : running ? (
              <div className="grid gap-1.5">
                <Progress value={setupPercent(setup)} aria-label="Setup progress" />
                <p className="text-xs text-muted-foreground tnum" aria-live="polite">{setupLine(setup)}</p>
              </div>
            ) : (
              <p>
                {setup?.phase === "failed" || setup?.phase === "stopped" ? (
                  <span className="block text-muted-foreground">{setupLine(setup)}</span>
                ) : null}
                The download is about {gigs(need)}
                {status.runtime && !status.runtime.ready ? " (ONNX Runtime " + status.runtime.version + " and the model)" : ""}
                ; this disk has {gigs(status.free_bytes)} free. It resumes if it is interrupted.
              </p>
            )}
            <p className="text-xs text-muted-foreground">
              The same from a terminal: <code className="font-mono">pulse voice setup</code>, <code className="font-mono">pulse voice status</code>.
              The <code className="font-mono">voice.model</code> and <code className="font-mono">voice.language</code> settings choose the model and the language it expects.
            </p>
          </div>
        )}
        <DialogFooter className="gap-2 sm:justify-between">
          <div className="flex gap-2">
            {status?.supported && (chosen?.have || status.runtime?.ready) && !running ? (
              <Button
                type="button"
                variant="ghost"
                size="sm"
                disabled={!!busy}
                onClick={() => void act("remove", () => writes.voiceRemove(model))}
              >
                <Trash2Icon /> Remove files
              </Button>
            ) : null}
          </div>
          <div className="flex gap-2">
            {running ? (
              <Button
                type="button"
                variant="outline"
                disabled={!!busy}
                onClick={() => void act("stop", () => writes.voiceSetupStop())}
              >
                {busy === "stop" ? <LoaderCircleIcon className="animate-spin" /> : null}
                Stop
              </Button>
            ) : status?.supported && !(chosen?.ready && status.runtime?.ready) ? (
              <Button
                type="button"
                disabled={!!busy || !model}
                onClick={() => void act("setup", () => writes.voiceSetup(model))}
              >
                {busy === "setup" ? <LoaderCircleIcon className="animate-spin" /> : null}
                {chosen?.have ? "Resume download" : "Download " + gigs(need)}
              </Button>
            ) : (
              <Button type="button" onClick={() => onOpenChange(false)}>
                Done
              </Button>
            )}
          </div>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
