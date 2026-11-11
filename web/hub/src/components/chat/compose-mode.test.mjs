import assert from "node:assert/strict";
import { test } from "node:test";
import {
  QUEUE_TITLE,
  SEND_TITLE,
  STEER_TITLE,
  blindModel,
  composeKeysHint,
  composeMode,
  composePlaceholder,
  sentLabel,
  showsNow,
  steeredOf,
  turnOpen,
  unseenImagesWarning,
} from "./compose-mode.ts";

test("a turn is open while the agent runs or waits on the owner", () => {
  assert.equal(turnOpen("running"), true);
  assert.equal(turnOpen("waiting"), true);
  assert.equal(turnOpen("idle"), false);
  assert.equal(turnOpen("closed"), false);
  assert.equal(turnOpen("ended"), false);
  assert.equal(turnOpen(""), false);
});

test("idle, Enter sends whatever the modifier; with a turn open Enter steers and the modifier queues", () => {
  assert.equal(composeMode("idle", false), "send");
  assert.equal(composeMode("idle", true), "send");
  assert.equal(composeMode("closed", true), "send");
  assert.equal(composeMode("running", false), "steer");
  assert.equal(composeMode("running", true), "queue");
  assert.equal(composeMode("waiting", false), "steer");
  assert.equal(composeMode("waiting", true), "queue");
});

test("a steer shows at once, since the hub wrote its row before answering", () => {
  assert.equal(showsNow("steer", "running", { queued: [], steered: ["go left"] }, "go left"), true);
  // An older hub's answer without the lists: trust the row.
  assert.equal(showsNow("steer", "running", {}, "go left"), true);
  assert.equal(showsNow("steer", "running", null, "go left"), true);
});

test("a steer the hub had to queue instead is listed as queued, not shown as sent", () => {
  assert.equal(showsNow("steer", "running", { queued: ["go left"], steered: [] }, "go left"), false);
  // Another message queued earlier does not hide this steer.
  assert.equal(showsNow("steer", "running", { queued: ["and the header"], steered: ["go left"] }, "go left"), true);
});

test("a send or a queue shows now only when the agent was idle", () => {
  assert.equal(showsNow("send", "idle", { queued: [], steered: [] }, "hi"), true);
  assert.equal(showsNow("send", "closed", {}, "hi"), true);
  assert.equal(showsNow("queue", "running", { queued: ["hi"], steered: [] }, "hi"), false);
  assert.equal(showsNow("queue", "waiting", {}, "hi"), false);
  assert.equal(showsNow("send", "running", {}, "hi"), false);
});

test("an older hub's live state has no steered list", () => {
  assert.deepEqual(steeredOf({ steered: ["a"] }), ["a"]);
  assert.deepEqual(steeredOf({}), []);
  assert.deepEqual(steeredOf(null), []);
  assert.deepEqual(steeredOf(undefined), []);
});

test("the words name the shortcuts and the two ways", () => {
  assert.match(STEER_TITLE, /Enter/);
  assert.doesNotMatch(STEER_TITLE, /Ctrl/);
  assert.match(QUEUE_TITLE, /⌘\/Ctrl\+Enter/);
  assert.match(SEND_TITLE, /Enter/);
  assert.match(composePlaceholder(true), /steer/i);
  assert.match(composePlaceholder(true), /⌘\/Ctrl\+Enter/);
  assert.doesNotMatch(composePlaceholder(false), /steer/i);
  assert.match(composeKeysHint(true), /steers/);
  assert.match(composeKeysHint(true), /queues/);
  assert.match(composeKeysHint(true), /Shift\+Enter/);
  assert.equal(composeKeysHint(false), "Enter sends, Shift+Enter for a new line.");
});

test("a steered row says so under the bubble", () => {
  assert.equal(sentLabel("sent", false), "Sent");
  assert.equal(sentLabel("answered", false), "Answered");
  assert.equal(sentLabel("sent", true), "Steered");
  assert.equal(sentLabel("answered", true), "Answered · steered");
});

const MODELS = [
  { id: "gpt-4.1", name: "GPT-4.1", default: true, vision: false },
  { id: "claude-sonnet-4.5", name: "Claude Sonnet 4.5", default: false, vision: true },
  { id: "old", name: "Old Hub Model", default: false },
];

test("a model whose list entry says it cannot see is named; one that sees or is unknown is not", () => {
  assert.equal(blindModel("gpt-4.1", MODELS), "GPT-4.1");
  assert.equal(blindModel("claude-sonnet-4.5", MODELS), "");
  // An older hub sends no vision field: unknown, so no warning.
  assert.equal(blindModel("old", MODELS), "");
  // A model the list does not name, or a list not loaded yet.
  assert.equal(blindModel("mystery", MODELS), "");
  assert.equal(blindModel("gpt-4.1", []), "");
});

test("the provider default is the model when the conversation picked none", () => {
  assert.equal(blindModel("", MODELS), "GPT-4.1");
  assert.equal(blindModel("", MODELS.map((m) => ({ ...m, default: m.id === "claude-sonnet-4.5" }))), "");
  assert.equal(blindModel("", MODELS.map((m) => ({ ...m, default: false }))), "");
  // A blind model without a display name is named by its id.
  assert.equal(blindModel("x", [{ id: "x", name: "", default: false, vision: false }]), "x");
});

test("the warning names the model and the image, and says what to do before sending", () => {
  assert.equal(unseenImagesWarning("", ["shot.png"]), "");
  assert.equal(unseenImagesWarning("GPT-4.1", []), "");
  assert.equal(
    unseenImagesWarning("GPT-4.1", ["shot.png"]),
    "GPT-4.1 cannot see images: shot.png goes as a file the agent can only read with its tools, not look at. Say in your message what it shows, or pick a model that takes images.",
  );
  assert.equal(
    unseenImagesWarning("GPT-4.1", ["a.png", "b.jpg"]),
    "GPT-4.1 cannot see images: 2 images go as files the agent can only read with its tools, not look at. Say in your message what they show, or pick a model that takes images.",
  );
});
