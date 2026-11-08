import assert from "node:assert/strict";
import { register } from "node:module";
import { test } from "node:test";

// job-repair.ts imports "./api" without an extension, the way Next resolves it;
// the hub's test loader resolves that (and transpiles the .ts) for node --test.
register("../components/project/tsx-test-loader.mjs", import.meta.url);

const {
  FAILED_JOBS_PAGE_SIZE,
  JOB_FAILURE_CLASS,
  JOB_REPAIR_CATEGORIES,
  JOB_REPAIR_MAX_SELECTION,
  JOB_REPAIR_SENTENCE,
  failedJobsPath,
  repairFailedJobs,
} = await import("./job-repair.ts");

// Stub fetch for one call; records every request and answers with `reply`.
async function withFetch(reply, run) {
  const saved = globalThis.fetch;
  const calls = [];
  globalThis.fetch = async (url, init) => {
    calls.push({ url, init });
    return new Response(JSON.stringify(reply), {
      status: 200,
      headers: { "Content-Type": "application/json" },
    });
  };
  try {
    await run(calls);
  } finally {
    globalThis.fetch = saved;
  }
}

const response = {
  requested: 2,
  queued: 1,
  skipped: 1,
  errors: 0,
  results: [
    { job_id: "job-a", outcome: "requeued", category: "eligible", reason: "", fix: "" },
    { job_id: "job-b", outcome: "skipped", category: "backoff", reason: "waiting", fix: "" },
  ],
};

test("every failure category has a label and a sentence for the job's card", () => {
  const categories = Object.keys(JOB_REPAIR_CATEGORIES);
  assert.deepEqual(Object.keys(JOB_REPAIR_SENTENCE).sort(), [...categories].sort());
  for (const c of categories) {
    assert.ok(JOB_REPAIR_CATEGORIES[c].trim().length > 0, c);
    assert.match(JOB_REPAIR_SENTENCE[c], /\.$/, c);
  }
  assert.deepEqual(Object.keys(JOB_FAILURE_CLASS).sort(), ["deterministic", "structural", "transient"]);
});

test("failedJobsPath pages the failed jobs, dropping what is not set", () => {
  assert.equal(failedJobsPath(), "jobs/failed?limit=" + FAILED_JOBS_PAGE_SIZE);
  assert.equal(failedJobsPath("demo-app"), "jobs/failed?project=demo-app&limit=50");
  assert.equal(
    failedJobsPath("demo app", "2026-11-08T09:00:00Z", 10),
    "jobs/failed?project=demo+app&before=2026-11-08T09%3A00%3A00Z&limit=10",
  );
  assert.equal(failedJobsPath("", "", 20), "jobs/failed?limit=20");
});

test("repairFailedJobs POSTs the selection with its intent header", async () => {
  await withFetch(response, async (calls) => {
    const got = await repairFailedJobs(["job-a", "job-b"], "demo-app");
    assert.deepEqual(got, response);
    assert.equal(calls.length, 1);
    const { url, init } = calls[0];
    assert.equal(url, "/api/v1/jobs/repair");
    assert.equal(init.method, "POST");
    assert.equal(init.cache, "no-store");
    assert.deepEqual(init.headers, { "Content-Type": "application/json", "X-Pulse-Job-Repair": "yes" });
    assert.deepEqual(JSON.parse(init.body), { job_ids: ["job-a", "job-b"], project: "demo-app" });
  });
});

test("repairFailedJobs leaves the project out when none is given", async () => {
  await withFetch(response, async (calls) => {
    const ids = Array.from({ length: JOB_REPAIR_MAX_SELECTION }, (_, i) => "job-" + i);
    await repairFailedJobs(ids);
    assert.equal(calls.length, 1);
    assert.deepEqual(JSON.parse(calls[0].init.body), { job_ids: ids });
  });
});

test("repairFailedJobs refuses a bad selection without calling the hub", async () => {
  const tooMany = Array.from({ length: JOB_REPAIR_MAX_SELECTION + 1 }, (_, i) => "job-" + i);
  const bad = [[], tooMany, ["job-a", "job-a"], ["job-a", ""], ["   "]];
  await withFetch(response, async (calls) => {
    for (const ids of bad) {
      await assert.rejects(repairFailedJobs(ids, "demo-app"), /Pick 1 to 5 different jobs/, JSON.stringify(ids));
    }
    assert.equal(calls.length, 0);
  });
});
