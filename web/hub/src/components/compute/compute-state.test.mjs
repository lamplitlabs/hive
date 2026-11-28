import assert from "node:assert/strict";
import { test } from "node:test";
import { ANY_PROJECT, ANY_PROJECT_WORD, choiceLabel, diskMeter, fitChoices, limitChoices, limitFieldHelp, noRentalWarning, refileOffer, requestState, keepAwakeOn, machineLabels, machineLogoKey, machineOs, machineProjects, meterPercent, meterTone, requestProject, requestProjectHint, requestReason, requestTitle, runnerState, slotsMeter, thisMachineOs, waitingNote } from "./compute-state.ts";

test("meterTone: ok below 70, warning below 90, failed from 90", () => {
  assert.equal(meterTone(0), "ok");
  assert.equal(meterTone(69.9), "ok");
  assert.equal(meterTone(70), "warning");
  assert.equal(meterTone(89), "warning");
  assert.equal(meterTone(90), "failed");
  assert.equal(meterTone(100), "failed");
  assert.equal(meterTone(null), "ok", "no data is an empty bar, not a warning");
  assert.equal(meterTone(NaN), "ok");
});

test("meterPercent rounds and clamps, null without a total", () => {
  assert.equal(meterPercent(1, 2), 50);
  assert.equal(meterPercent(2, 3), 67);
  assert.equal(meterPercent(5, 2), 100);
  assert.equal(meterPercent(0, 4), 0);
  assert.equal(meterPercent(1, 0), null);
  assert.equal(meterPercent(null, 4), null);
});

test("requestTitle reads like the spec", () => {
  assert.equal(requestTitle({ hours: 8, project: "demo-lib" }), "Extra machine for 8 h · demo-lib");
  assert.equal(requestTitle({ hours: 1.5, project: "demo-app" }), "Extra machine for 1.5 h · demo-app");
  assert.equal(requestTitle({ hours: 4, project: "" }), "Extra machine for 4 h");
  assert.equal(requestTitle({ hours: 4, project: "commons" }), "Extra machine for 4 h · Any project", "a machine for every project never shows the shared board's slug");
});

test("requestProject names the project, or Any project for a machine every project can use", () => {
  assert.equal(ANY_PROJECT, "commons", "the brain reads the shared board as every project (hive_tasks::ANY_PROJECT)");
  assert.equal(requestProject(ANY_PROJECT), ANY_PROJECT_WORD);
  assert.equal(requestProject("commons", () => "Shared board"), "Any project", "the shared board's own name is not used for a request");
  assert.equal(requestProject("demo-app"), "demo-app");
  assert.equal(requestProject("demo-app", (s) => (s === "demo-app" ? "Demo App" : "")), "Demo App");
  assert.equal(requestProject("demo-app", () => "  "), "demo-app", "an empty name falls back to the slug");
  assert.equal(requestProject(" demo-lib "), "demo-lib");
  assert.equal(requestProject(""), "");
  assert.equal(requestProject(null), "");
});

test("requestProjectHint says what the choice means for the machine", () => {
  assert.equal(requestProjectHint(""), "Whose work it runs. Any project lets every project use it.");
  assert.equal(requestProjectHint(ANY_PROJECT), "Every project's work can run on it.");
  assert.equal(requestProjectHint("demo-app", "Demo App"), "Only Demo App's work runs on it.");
  assert.equal(requestProjectHint("demo-app"), "Only demo-app's work runs on it.");
});

test("runnerState: paused beats running, unknown is stopped", () => {
  assert.equal(runnerState({ daemon: { state: "running" }, runner: { allowed: true } }), "running");
  assert.equal(runnerState({ daemon: { state: "alive" }, runner: { allowed: true } }), "running");
  assert.equal(runnerState({ daemon: { state: "running" }, runner: { allowed: false } }), "paused");
  assert.equal(runnerState({ daemon: { state: "never started" }, runner: { allowed: true } }), "stopped");
  assert.equal(runnerState({ daemon: { state: "dead" } }), "stopped");
  assert.equal(runnerState(null), "stopped");
});

test("machineOs reads the offers line first, then hints", () => {
  assert.equal(machineOs({ offers: "macos arm64 xcode" }), "macos");
  assert.equal(machineOs({ offers: "linux amd64 docker" }), "linux");
  assert.equal(machineOs({ offers: "-", probe_note: "this machine" }, "Manish's Mac"), "macos");
  assert.equal(machineOs({ offers: "", kind: "ssh" }), "linux", "ssh assumes linux like the runner");
  assert.equal(machineOs({ offers: "", kind: "local" }), null);
});

test("thisMachineOs: the machine's own row, then its host name, then the browser", () => {
  const mac = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)";
  assert.equal(thisMachineOs({ kind: "local", offers: "linux amd64" }, "", mac), "linux", "the row wins over the browser");
  assert.equal(thisMachineOs({ kind: "local", offers: "-" }, "Manish's Mac", ""), "macos");
  assert.equal(thisMachineOs(null, "", mac), "macos", "the console is mostly opened on the machine itself");
  assert.equal(thisMachineOs({ kind: "local", offers: "" }, "", "Mozilla/5.0 (X11; Linux x86_64)"), null);
});

test("machineLogoKey: system for local/ssh, provider for cloud", () => {
  assert.equal(machineLogoKey({ kind: "local", offers: "macos arm64" }), "macos");
  assert.equal(machineLogoKey({ kind: "ssh", offers: "linux amd64 ubuntu" }), "ubuntu");
  assert.equal(machineLogoKey({ kind: "ssh", offers: "" }), "linux");
  assert.equal(machineLogoKey({ kind: "cloud" }, "azure"), "azure");
  assert.equal(machineLogoKey({ kind: "vm" }, "gcp"), "google cloud");
  assert.equal(machineLogoKey({ kind: "local", offers: "-" }), null);
});

test("machineLabels drops the system words and the reserved marker", () => {
  assert.deepEqual(machineLabels("macos arm64 xcode docker (reserved)"), ["xcode", "docker"]);
  assert.deepEqual(machineLabels("-"), []);
  assert.deepEqual(machineLabels(""), []);
});

test("machineProjects: * and empty are every project", () => {
  assert.deepEqual(machineProjects("*"), []);
  assert.deepEqual(machineProjects(""), []);
  assert.deepEqual(machineProjects("demo-app, demo-lib"), ["demo-app", "demo-lib"]);
});

test("slotsMeter fills with the used slots", () => {
  assert.deepEqual(slotsMeter({ workers: 4, running: 0, free: 4 }), { used: 0, total: 4, pct: 0, text: "0 of 4 in use" });
  assert.deepEqual(slotsMeter({ workers: 4, running: 3, free: 1 }), { used: 3, total: 4, pct: 75, text: "3 of 4 in use" });
  assert.deepEqual(slotsMeter({ workers: 2, running: 1 }), { used: 1, total: 2, pct: 50, text: "1 of 2 in use" });
  assert.deepEqual(slotsMeter({ workers: 0 }), { used: 0, total: 0, pct: null, text: "No slots" });
});

test("diskMeter parses the daemon's disk line and is — without one", () => {
  assert.deepEqual(diskMeter("disk: 252 MB free, needs 1024; 6 projects waiting"), { pct: 75, text: "252 MB free" });
  assert.deepEqual(diskMeter("disk: 0 MB free, needs 1024; all projects waiting"), { pct: 100, text: "0 MB free" });
  assert.deepEqual(diskMeter(undefined), { pct: null, text: "—" });
  assert.deepEqual(diskMeter("bulk: something else"), { pct: null, text: "—" });
});

test("keepAwakeOn", () => {
  assert.equal(keepAwakeOn("on"), true);
  assert.equal(keepAwakeOn("always"), true);
  assert.equal(keepAwakeOn("off"), false);
  assert.equal(keepAwakeOn(""), false);
  assert.equal(keepAwakeOn(undefined), false);
});

test("requestReason keeps the first sentence on one line", () => {
  assert.equal(requestReason("fuzzing the v2 parser overnight"), "fuzzing the v2 parser overnight");
  assert.equal(requestReason("The matrix needs eight cores. It runs for four hours."), "The matrix needs eight cores.");
  assert.equal(requestReason("  spaced   out  "), "spaced out");
  assert.equal(requestReason(""), "");
  assert.equal(requestReason("x".repeat(200), 20).length, 20);
});

test("the Waiting for you line names what waits: a request, or only Hive's cleanup question", () => {
  assert.equal(waitingNote(1, false), "An agent asked for an extra machine. It starts only if you approve.");
  assert.equal(waitingNote(2, true), "An agent asked for an extra machine. It starts only if you approve.");
  assert.equal(waitingNote(0, true), "Hive asks before it cleans up after other agent programs on this machine.");
  assert.equal(waitingNote(0, false), "");
});


// TASK-165 item 1: over a limit the owner gets three answers, the first recommended, no free text.
test("limitChoices: Rent it anyway (Recommended) / raise to N / Do not rent", () => {
  const hit = { setting: "compute.max_hours_per_day", limit: 24, used: 12, asks: 16, raise_to: 28 };
  assert.deepEqual(limitChoices(hit).map(choiceLabel), ["Rent it anyway (Recommended)", "Rent it and raise the limit to 28", "Do not rent"]);
});

// TASK-165 item 2: 0 is explained beside the field, none reads as no limit, and a change
// that leaves no rental possible says so; refused requests are offered again.
test("limitFieldHelp explains 0 and none", () => {
  assert.match(limitFieldHelp("0"), /^0 - Hive rents nothing/);
  assert.match(limitFieldHelp("none"), /^No limit/);
  assert.match(limitFieldHelp("2"), /never refused/);
});

test("noRentalWarning says when no rental is possible", () => {
  assert.match(noRentalWarning("0", "24"), /rents no machine by itself/);
  assert.match(noRentalWarning("2", "0"), /rents no machine by itself/);
  assert.equal(noRentalWarning("2", "none"), "");
  assert.equal(refileOffer(0), "");
  assert.equal(refileOffer(2), "2 requests were not rented today because of a limit. Rent them now?");
});

// TASK-165 item 3: no waiting job fits -> Do not rent (Recommended), Rent it anyway, and the
// Linux switch with its risk when the project's needs name macos.
test("fitChoices: no fitting job recommends not renting and names the Linux risk", () => {
  const p = { fit: { fits: 0, waiting: 1, blocked: [{ project: "brain", jobs: 1, missing: ["macos"] }] }, os_needs: ["macos"], linux_risk: "brain's jobs then run on Linux machines too" };
  const c = fitChoices(p);
  assert.deepEqual(c.map(choiceLabel), ["Do not rent (Recommended)", "Rent it anyway", "Let project jobs run on Linux"]);
  assert.match(c[2].detail, /^Risk: brain's jobs then run on Linux/);
  assert.equal(fitChoices({ ...p, fit: { fits: 2, waiting: 3, blocked: [] } }), null, "a fitting job needs no question");
});

// TASK-165 item 4: a request a limit refused reads "Not rented", not "Failed".
test("requestState: a limit refusal is Not rented, a real failure stays Failed", () => {
  const hit = { setting: "compute.max_hours_per_day", limit: 24, used: 12, asks: 16, raise_to: 28 };
  const r = requestState({ status: "failed", note: "Not rented: today's limit of 24 hours is used (12 used, this asks 16)", limit_hit: hit });
  assert.deepEqual([r.label, r.bad, r.lead], ["Not rented", false, ""]);
  assert.deepEqual(requestState({ status: "failed", note: "az said no" }), { badge: "failed", lead: "Why it failed: ", bad: true });
});
