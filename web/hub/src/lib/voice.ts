// The microphone in the composer, the part that needs no browser: what the
// hub's voice status means for the button, the words under the setup bar,
// how the transcript lands in the text, and the PCM the recorder sends.
//
// The hub route (`/api/v1/voice`) runs NVIDIA's Nemotron streaming ASR on
// this machine through ONNX Runtime - the model GitHub Copilot's CLI uses
// for its /voice - so the recording never leaves the computer. The first
// press fetches the runtime and the model (about 2.5 GB) into PULSE_HOME.

export interface VoiceFile {
  name: string;
  size: number;
  have: number;
  ready: boolean;
}

export interface VoiceModel {
  id: string;
  title: string;
  languages: string;
  size: number;
  have: number;
  ready: boolean;
  files: VoiceFile[];
}

export interface VoiceSetup {
  // runtime | model | done | failed | stopped
  phase: string;
  model: string;
  file: string;
  done: number;
  total: number;
  error: string;
  started_at: string;
  finished_at: string;
}

export interface VoiceStatus {
  supported: boolean;
  platform: string;
  runtime: { version: string; ready: boolean; size: number } | null;
  model: VoiceModel;
  language: string;
  models: VoiceModel[];
  ready: boolean;
  loaded: string | null;
  free_bytes: number;
  setup: VoiceSetup | null;
  root: string;
  setting_up: boolean;
  idle_release_minutes: number;
  languages: string[];
  max_seconds: number;
}

export interface Transcript {
  text: string;
  model: string;
  language: string;
  seconds: number;
  took_ms: number;
}

// What the button is for right now.
export type MicState =
  | "unsupported" // no runtime build for this machine: the button explains
  | "setup" // files missing: a press opens the setup dialog
  | "fetching" // the setup runs: the button shows progress
  | "ready" // a press starts recording
  | "recording"
  | "transcribing";

export function micState(
  status: VoiceStatus | undefined,
  recording: boolean,
  transcribing: boolean,
): MicState {
  if (recording) return "recording";
  if (transcribing) return "transcribing";
  if (!status) return "ready";
  if (!status.supported) return "unsupported";
  if (status.setting_up) return "fetching";
  if (!status.ready) return "setup";
  return "ready";
}

// The word on the button's tooltip.
export function micTitle(state: MicState, status?: VoiceStatus): string {
  switch (state) {
    case "unsupported":
      return "Dictation needs an Apple-silicon Mac or an x86-64/arm64 Linux machine.";
    case "setup":
      return "Dictate (sets up the speech model first, about " + gigs(status?.model.size ?? 0) + ")";
    case "fetching":
      return "Fetching the speech model… " + setupLine(status?.setup ?? null);
    case "recording":
      return "Stop and transcribe";
    case "transcribing":
      return "Transcribing…";
    default:
      return "Dictate (transcribed on this machine, nothing leaves it)";
  }
}

export function gigs(bytes: number): string {
  if (bytes >= 1024 * 1024 * 1024) return (bytes / (1024 * 1024 * 1024)).toFixed(1) + " GB";
  if (bytes >= 1024 * 1024) return Math.round(bytes / (1024 * 1024)) + " MB";
  if (bytes >= 1024) return Math.round(bytes / 1024) + " kB";
  return bytes + " B";
}

// The line under the progress bar while a setup runs or after it ended.
export function setupLine(s: VoiceSetup | null): string {
  if (!s || !s.phase) return "";
  const share = s.total > 0 ? Math.min(100, Math.floor((s.done / s.total) * 100)) : 0;
  switch (s.phase) {
    case "runtime":
      return "ONNX Runtime " + (s.file ? "(" + s.file + ") " : "") + gigs(s.done) + " of " + gigs(s.total) + " · " + share + "%";
    case "model":
      return s.model + (s.file ? " · " + s.file : "") + " · " + gigs(s.done) + " of " + gigs(s.total) + " · " + share + "%";
    case "done":
      return "Ready: " + s.model + " is on this machine.";
    case "stopped":
      return "Stopped at " + gigs(s.done) + " of " + gigs(s.total) + "; the parts stay and a new setup resumes.";
    case "failed":
      return "Failed: " + (s.error || "unknown error");
    default:
      return s.phase;
  }
}

export function setupPercent(s: VoiceSetup | null): number {
  if (!s || !s.total) return 0;
  return Math.max(0, Math.min(100, (s.done / s.total) * 100));
}

// Puts `spoken` into `text` where the caret is (or over the selection),
// with the spaces a sentence needs around it; returns the new text and
// where the caret goes. `start`/`end` are the textarea's selection.
export function insertSpoken(
  text: string,
  start: number,
  end: number,
  spoken: string,
): { text: string; caret: number } {
  const s = spoken.trim();
  if (!s) return { text, caret: end };
  const lo = Math.max(0, Math.min(start, end, text.length));
  const hi = Math.max(lo, Math.min(Math.max(start, end), text.length));
  const before = text.slice(0, lo);
  const after = text.slice(hi);
  const lead = before && !/\s$/.test(before) ? " " : "";
  const tail = after && !/^\s/.test(after) ? " " : "";
  const inserted = lead + s + tail;
  return { text: before + inserted + after, caret: lo + lead.length + s.length };
}

// Float samples from the recorder (-1..1) to the little-endian 16-bit PCM
// the hub reads, clipped.
export function pcm16(samples: Float32Array): Uint8Array {
  const out = new Uint8Array(samples.length * 2);
  const view = new DataView(out.buffer);
  for (let i = 0; i < samples.length; i++) {
    const v = Math.max(-1, Math.min(1, samples[i]));
    view.setInt16(i * 2, v < 0 ? v * 0x8000 : v * 0x7fff, true);
  }
  return out;
}

// Linear resample from the device's rate to 16 kHz, so a browser that
// refuses a 16 kHz AudioContext still produces what the model takes.
export function resample(samples: Float32Array, from: number, to: number): Float32Array {
  if (from === to || !samples.length) return samples;
  const ratio = from / to;
  const n = Math.max(1, Math.round(samples.length / ratio));
  const out = new Float32Array(n);
  for (let i = 0; i < n; i++) {
    const pos = i * ratio;
    const i0 = Math.floor(pos);
    const i1 = Math.min(samples.length - 1, i0 + 1);
    const frac = pos - i0;
    out[i] = samples[i0] * (1 - frac) + samples[i1] * frac;
  }
  return out;
}

export const VOICE_SAMPLE_RATE = 16000;

// A recording the hub rejects before it is sent: too short to carry a word,
// or past the route's limit.
export function recordingProblem(seconds: number, maxSeconds: number): string {
  if (seconds < 0.3) return "Nothing recorded; hold the microphone a moment longer.";
  if (seconds > maxSeconds) return "The recording is longer than " + maxSeconds + " seconds; speak in shorter parts.";
  return "";
}
