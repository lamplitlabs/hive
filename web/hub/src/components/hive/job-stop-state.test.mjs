import assert from "node:assert/strict";
import { test } from "node:test";
import {
  LOCAL_EXECUTOR,
  STOP_DISMISS,
  machineName,
  stopActionLabel,
  stopButtonLabel,
  stopButtonTitle,
  stopDialogDetail,
  stopDialogTitle,
  stopPendingMessage,
  stopRequeueLabel,
  stopShown,
  stopSuccessMessage,
} from "./job-stop-state.ts";

test("machineName names a paired or rented machine and calls this one this machine", () => {
  assert.equal(machineName("hands-cr-1"), "hands-cr-1");
  assert.equal(machineName(" hands-cr-1 "), "hands-cr-1");
  for (const here of [LOCAL_EXECUTOR, "", null, undefined]) assert.equal(machineName(here), "this machine", String(here));
});

test("the Stop shows only for a job being worked on", () => {
  assert.equal(stopShown("leased"), true);
  assert.equal(stopShown("running"), true, "a harness session's status");
  for (const s of ["queued", "blocked", "done", "failed", "cancelled", "", null, undefined]) assert.equal(stopShown(s), false, String(s));
});

test("the button and its tip name the machine", () => {
  assert.equal(stopButtonLabel(false), "Stop");
  assert.equal(stopButtonLabel(true), "Stopping…");
  assert.equal(stopButtonTitle("hands-cr-1"), "Stop this run on hands-cr-1");
  assert.equal(stopButtonTitle("local"), "Stop this run on this machine");
});

test("the dialog asks for the right stop on the named machine", () => {
  assert.equal(stopDialogTitle("hands-cr-1", false), "Stop the run on hands-cr-1?");
  assert.equal(stopDialogTitle("hands-cr-1", true), "Stop the run on hands-cr-1 and requeue the job?");
  assert.equal(stopActionLabel(false), "Stop the run");
  assert.equal(stopActionLabel(true), "Stop and requeue");
  assert.match(stopRequeueLabel(), /^Stop and requeue/);
  assert.equal(STOP_DISMISS, "Keep running", "the dismiss button can't be confused with the stop");
});

test("the dialog says what the stop does to the run's commits", () => {
  const cancel = stopDialogDetail(false);
  assert.match(cancel, /ends cancelled/);
  assert.match(cancel, /commits are dropped/);
  assert.match(cancel, /checking or landing/, "even during the end check or landing");
  assert.match(cancel, /Stop and requeue/, "points at the choice that keeps the job");
  const requeue = stopDialogDetail(true);
  assert.match(requeue, /back to the queue/);
  assert.match(requeue, /no attempt/);
  assert.match(requeue, /dropped/);
  assert.match(requeue, /kept and the next run checks and lands them/);
});

test("the toast names the run and its machine from the hub's answer", () => {
  assert.equal(
    stopSuccessMessage({ job: "job-1", run: "run-away", executor: "hands-cr-1", requeue: false }),
    "Stopping run run-away on hands-cr-1; the job ends cancelled once it has stopped (within a few seconds).",
  );
  assert.equal(
    stopSuccessMessage({ job: "job-1", run: "run-here", executor: "local", requeue: true }),
    "Stopping run run-here on this machine; the job goes back to the queue once it has stopped (within a few seconds).",
  );
  assert.equal(stopPendingMessage("hands-cr-1"), "Stopping the run on hands-cr-1");
});
