import test from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, mkdir, writeFile, utimes } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import {
  artifactMime,
  createJobArtifactsMock,
  safeArtifactName,
  sortNewest,
} from "./job-artifacts.mjs";

async function folder() {
  const root = await mkdtemp(join(tmpdir(), "job-artifacts-"));
  const dir = join(root, "jobs", "job-1", "artifacts");
  await mkdir(dir, { recursive: true });
  await writeFile(join(dir, "report.md"), "# report\n");
  await writeFile(join(dir, "shot.png"), Buffer.from([0x89, 0x50, 0x4e, 0x47]));
  await writeFile(join(dir, ".DS_Store"), "x");
  const older = new Date("2026-09-01T19:07:00Z");
  const newer = new Date("2026-09-01T19:08:30Z");
  await utimes(join(dir, "report.md"), older, older);
  await utimes(join(dir, "shot.png"), newer, newer);
  return root;
}

test("the listing has the chat shape, newest first, and skips hidden files", async () => {
  const mock = createJobArtifactsMock({ root: await folder() });
  const l = await mock.list("job-1");
  assert.equal(l.job, "job-1");
  assert.deepEqual(
    l.artifacts.map((a) => a.name),
    ["shot.png", "report.md"],
  );
  assert.equal(l.artifacts[0].size, 4);
  assert.equal(l.artifacts[0].modified, "2026-09-01T19:08:30Z");
  assert.equal(l.artifacts[1].modified, "2026-09-01T19:07:00Z");
  assert.equal(await mock.count("job-1"), 2);
});

test("a job without a folder is unknown; a bad id never leaves the jobs tree", async () => {
  const mock = createJobArtifactsMock({ root: await folder() });
  assert.equal(await mock.list("job-2"), null);
  assert.equal(await mock.count("job-2"), 0);
  assert.equal(await mock.list("../tasks"), null);
  assert.equal(await mock.list(""), null);
  assert.equal(await mock.file("job-1", "../../tasks/TASK-001.json"), null);
  assert.equal(await mock.file("job-1", ".DS_Store"), null);
});

test("a file comes back with its bytes and mime", async () => {
  const mock = createJobArtifactsMock({ root: await folder() });
  const png = await mock.file("job-1", "shot.png");
  assert.equal(png.type, "image/png");
  assert.equal(png.bytes.length, 4);
  const md = await mock.file("job-1", "report.md");
  assert.equal(md.type, "text/markdown; charset=utf-8");
  assert.equal(md.bytes.toString(), "# report\n");
  assert.equal(await mock.file("job-1", "gone.txt"), null);
});

test("the task detail's jobs gain a count only where the folder has files", async () => {
  const mock = createJobArtifactsMock({ root: await folder() });
  const jobs = await mock.decorate([
    { id: "job-1", title: "a" },
    { id: "job-2", title: "b" },
  ]);
  assert.equal(jobs[0].artifacts, 2);
  assert.equal("artifacts" in jobs[1], false);
  assert.deepEqual(await mock.decorate(undefined), []);
});

test("mime by extension, names as the hub accepts them, newest first with a name tiebreak", () => {
  assert.equal(artifactMime("a.PNG"), "image/png");
  assert.equal(artifactMime("page.html"), "text/html; charset=utf-8");
  assert.equal(artifactMime("noext"), "application/octet-stream");
  assert.equal(safeArtifactName("shot.png"), true);
  assert.equal(safeArtifactName("a/b.png"), false);
  assert.equal(safeArtifactName(".hidden"), false);
  assert.equal(safeArtifactName(""), false);
  assert.deepEqual(
    sortNewest([
      { name: "b", modified: "2026-01-01T00:00:00Z" },
      { name: "a", modified: "2026-01-01T00:00:00Z" },
      { name: "c", modified: "2026-01-02T00:00:00Z" },
    ]).map((a) => a.name),
    ["c", "a", "b"],
  );
});

test("the shipped fixture folder has one image and one report for a TASK-002 job", async () => {
  const root = new URL("./api", import.meta.url).pathname;
  const mock = createJobArtifactsMock({ root });
  const l = await mock.list("job-20260901T190200Z-87e2d1");
  assert.deepEqual(
    l.artifacts.map((a) => a.name).sort(),
    ["checkout-total.png", "test-run.md"],
  );
});
