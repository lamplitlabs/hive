// The state behind the console's Stop for a job's run, as a pure module so
// node tests can run it straight from the source (like restart-now-state.ts).
// `job-stop.tsx` renders what these return. The hub answers POST
// jobs/{id}/stop {requeue} with {job, run, executor, requeue}; the run's
// watcher, on whatever machine runs it (this one, a paired or a rented
// one), stops it within a few seconds (hive_board::stop_job, TASK-144).

export interface JobStopResult {
  job: string;
  run: string;
  executor: string;
  requeue: boolean;
}

// The runner's name for this machine (hive_board::LOCAL_EXECUTOR).
export const LOCAL_EXECUTOR = "local";

// The machine a run is on, as the owner reads it: a paired or rented
// machine by its name, this one as "this machine".
export function machineName(executor: string | null | undefined): string {
  const name = (executor ?? "").trim();
  return !name || name === LOCAL_EXECUTOR ? "this machine" : name;
}

// Whether the job has a run the Stop can reach: a leased job is the one
// being worked on (the runner's status while its run goes). A job whose
// run is still starting is refused by the hub in its own words.
export function stopShown(jobStatus: string | null | undefined): boolean {
  return jobStatus === "leased" || jobStatus === "running";
}

export function stopButtonLabel(pending: boolean): string {
  return pending ? "Stopping…" : "Stop";
}

// The tooltip / aria label names the machine, so a Stop on a rented
// machine never reads like one on this machine.
export function stopButtonTitle(executor: string | null | undefined): string {
  return "Stop this run on " + machineName(executor);
}

export function stopDialogTitle(executor: string | null | undefined, requeue: boolean): string {
  const on = machineName(executor);
  return requeue ? "Stop the run on " + on + " and requeue the job?" : "Stop the run on " + on + "?";
}

// What a stop does to the job and to the commits its run made, in the
// order the runner decides it (crates/hive-runner/src/run.rs): without
// requeue nothing the run committed lands, even when the stop arrives
// during its end check or landing; with requeue, work the run had finished
// and was checking or landing is kept and the next run lands it.
export function stopDialogDetail(requeue: boolean): string {
  return requeue
    ? "The job goes back to the queue and the stop costs it no attempt. If the agent is still working, what it committed is dropped and the next run starts the job again; if the run is already checking or landing finished work, those commits are kept and the next run checks and lands them."
    : "The job ends cancelled by you. Nothing the run committed lands: its commits are dropped, even when the run is already checking or landing them. To keep the job, choose Stop and requeue.";
}

export function stopRequeueLabel(): string {
  return "Stop and requeue: put the job back in the queue to run again";
}

export function stopActionLabel(requeue: boolean): string {
  return requeue ? "Stop and requeue" : "Stop the run";
}

export const STOP_DISMISS = "Keep running";

// The toast once the hub wrote the stop: it names the run and its machine
// ("Stopping run R on hands-cr-1") and what follows.
export function stopSuccessMessage(r: JobStopResult): string {
  const then = r.requeue ? "the job goes back to the queue" : "the job ends cancelled";
  return "Stopping run " + r.run + " on " + machineName(r.executor) + "; " + then + " once it has stopped (within a few seconds).";
}

// Before the hub answered: the toast while the write is in flight.
export function stopPendingMessage(executor: string | null | undefined): string {
  return "Stopping the run on " + machineName(executor);
}
