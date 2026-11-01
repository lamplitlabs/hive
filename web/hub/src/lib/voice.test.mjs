import assert from "node:assert/strict";
import { test } from "node:test";
import {
  gigs,
  insertSpoken,
  micState,
  micTitle,
  pcm16,
  recordingProblem,
  resample,
  setupLine,
  setupPercent,
} from "./voice.ts";

const model = { id: "nemotron-3.5-asr-streaming-0.6b", title: "Nemotron", languages: "25+", size: 2_599_375_762, have: 0, ready: false, files: [] };
const base = {
  supported: true,
  platform: "osx-arm64",
  runtime: { version: "1.31.1", ready: false, size: 44_797_256 },
  model,
  language: "auto",
  models: [model],
  ready: false,
  loaded: null,
  free_bytes: 60e9,
  setup: null,
  root: "/home/owner/.pulse/voice",
  setting_up: false,
  idle_release_minutes: 10,
  languages: ["auto", "en-US"],
  max_seconds: 300,
};

test("the button's state follows the hub's status, then the recorder", () => {
  assert.equal(micState(undefined, false, false), "ready");
  assert.equal(micState({ ...base, supported: false }, false, false), "unsupported");
  assert.equal(micState(base, false, false), "setup");
  assert.equal(micState({ ...base, setting_up: true }, false, false), "fetching");
  assert.equal(micState({ ...base, ready: true }, false, false), "ready");
  assert.equal(micState({ ...base, ready: true }, true, false), "recording");
  assert.equal(micState(base, false, true), "transcribing");
  assert.equal(micState(base, true, true), "recording");
});

test("the tooltip names the download before setup and privacy after", () => {
  assert.equal(micTitle("setup", base), "Dictate (sets up the speech model first, about 2.4 GB)");
  assert.match(micTitle("ready"), /nothing leaves it/);
  assert.match(micTitle("unsupported"), /Apple-silicon/);
  assert.equal(micTitle("recording"), "Stop and transcribe");
});

test("sizes read as a person says them", () => {
  assert.equal(gigs(980), "980 B");
  assert.equal(gigs(251_056), "245 kB");
  assert.equal(gigs(44_797_256), "43 MB");
  assert.equal(gigs(2_599_375_762), "2.4 GB");
});

test("the setup line says what is being fetched and how far it is", () => {
  assert.equal(setupLine(null), "");
  assert.equal(setupLine({ phase: "", model: "", file: "", done: 0, total: 0, error: "", started_at: "", finished_at: "" }), "");
  const s = { phase: "model", model: "nemotron-3.5-asr-streaming-0.6b", file: "encoder.onnx.data", done: 1_300_000_000, total: 2_600_000_000, error: "", started_at: "", finished_at: "" };
  assert.equal(setupLine(s), "nemotron-3.5-asr-streaming-0.6b · encoder.onnx.data · 1.2 GB of 2.4 GB · 50%");
  assert.equal(setupPercent(s), 50);
  assert.equal(setupLine({ ...s, phase: "runtime", file: "libonnxruntime.1.31.1.dylib", done: 10_000_000, total: 2_600_000_000 }), "ONNX Runtime (libonnxruntime.1.31.1.dylib) 10 MB of 2.4 GB · 0%");
  assert.equal(setupLine({ ...s, phase: "done" }), "Ready: nemotron-3.5-asr-streaming-0.6b is on this machine.");
  assert.match(setupLine({ ...s, phase: "stopped" }), /^Stopped at 1\.2 GB of 2\.4 GB; the parts stay/);
  assert.equal(setupLine({ ...s, phase: "failed", error: "disk full" }), "Failed: disk full");
  assert.equal(setupPercent(null), 0);
  assert.equal(setupPercent({ ...s, done: 5_000_000_000 }), 100);
});

test("the transcript lands at the caret with the spaces a sentence needs", () => {
  assert.deepEqual(insertSpoken("", 0, 0, "hello there"), { text: "hello there", caret: 11 });
  assert.deepEqual(insertSpoken("Fix the", 7, 7, "tests"), { text: "Fix the tests", caret: 13 });
  assert.deepEqual(insertSpoken("Fix the ", 8, 8, "tests"), { text: "Fix the tests", caret: 13 });
  // In the middle: a space on both sides when the neighbours have none.
  assert.deepEqual(insertSpoken("Fix thetests", 7, 7, "failing"), { text: "Fix the failing tests", caret: 15 });
  // A selection is replaced.
  assert.deepEqual(insertSpoken("Fix the old tests", 8, 11, "new"), { text: "Fix the new tests", caret: 11 });
  // Reversed selection and out-of-range offsets are tolerated.
  assert.deepEqual(insertSpoken("abc", 2, 1, "X"), { text: "a X c", caret: 3 });
  assert.deepEqual(insertSpoken("abc", 10, 10, "X"), { text: "abc X", caret: 5 });
  // Nothing spoken changes nothing.
  assert.deepEqual(insertSpoken("abc", 1, 1, "   "), { text: "abc", caret: 1 });
  // Whitespace around the words is trimmed first.
  assert.deepEqual(insertSpoken("", 0, 0, "  hi  "), { text: "hi", caret: 2 });
});

test("samples become clipped little-endian 16-bit PCM", () => {
  const bytes = pcm16(new Float32Array([0, 0.5, -0.5, 1, -1, 2, -2]));
  assert.equal(bytes.length, 14);
  const v = new DataView(bytes.buffer);
  assert.equal(v.getInt16(0, true), 0);
  assert.equal(v.getInt16(2, true), Math.trunc(0.5 * 0x7fff));
  assert.equal(v.getInt16(4, true), -0x4000);
  assert.equal(v.getInt16(6, true), 0x7fff);
  assert.equal(v.getInt16(8, true), -0x8000);
  assert.equal(v.getInt16(10, true), 0x7fff);
  assert.equal(v.getInt16(12, true), -0x8000);
});

test("resampling keeps the length in proportion and the same rate untouched", () => {
  const src = new Float32Array(48000).map((_, i) => Math.sin(i / 10));
  const out = resample(src, 48000, 16000);
  assert.equal(out.length, 16000);
  assert.ok(Math.abs(out[0] - src[0]) < 1e-6);
  assert.ok(Math.abs(out[1] - src[3]) < 1e-6);
  assert.equal(resample(src, 16000, 16000), src);
  assert.equal(resample(new Float32Array(0), 48000, 16000).length, 0);
});

test("a recording too short or too long is refused before it is sent", () => {
  assert.match(recordingProblem(0.1, 300), /Nothing recorded/);
  assert.equal(recordingProblem(2, 300), "");
  assert.match(recordingProblem(301, 300), /longer than 300 seconds/);
});
