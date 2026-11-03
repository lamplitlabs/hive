// The files a fleet job left in its artifacts folder, for the offline
// console: GET jobs/<id>/artifacts lists mock/api/jobs/<id>/artifacts/
// (newest first, the shape of a conversation's listing) and
// GET jobs/<id>/artifacts/<name> serves one file with its mime, so the
// task view's evidence tab can be exercised without a hub. The task
// detail's jobs carry `artifacts: N` from the same folders.

import { readFile, readdir, stat } from "node:fs/promises";
import { join, normalize } from "node:path";

export const ARTIFACT_MIME = {
  png: "image/png",
  jpg: "image/jpeg",
  jpeg: "image/jpeg",
  gif: "image/gif",
  webp: "image/webp",
  svg: "image/svg+xml",
  html: "text/html; charset=utf-8",
  htm: "text/html; charset=utf-8",
  md: "text/markdown; charset=utf-8",
  txt: "text/plain; charset=utf-8",
  csv: "text/csv; charset=utf-8",
  json: "application/json",
  pdf: "application/pdf",
};

export function artifactMime(name) {
  const dot = name.lastIndexOf(".");
  const ext = dot >= 0 ? name.slice(dot + 1).toLowerCase() : "";
  return ARTIFACT_MIME[ext] ?? "application/octet-stream";
}

// A file name as the hub accepts it: one path segment, no dot-dirs, nothing
// hidden. Anything else is not a file of the folder.
export function safeArtifactName(name) {
  return (
    typeof name === "string" &&
    name.length > 0 &&
    name.length <= 255 &&
    !name.includes("/") &&
    !name.includes("\\") &&
    !name.startsWith(".")
  );
}

// Newest first, then by name so two files of one second keep one order.
export function sortNewest(entries) {
  return [...entries].sort(
    (a, b) => b.modified.localeCompare(a.modified) || a.name.localeCompare(b.name),
  );
}

const iso = (d) => new Date(d).toISOString().replace(/\.\d{3}Z$/, "Z");

export function createJobArtifactsMock({ root }) {
  const folder = (jobId) => normalize(join(root, "jobs", jobId, "artifacts"));
  const inside = (p) => p.startsWith(normalize(join(root, "jobs")) + "/");

  // The listing: null when the job has no folder (the route answers an
  // empty list for a job the hub knows, 404 for one it does not; the mock
  // knows only the jobs with a folder, so null is its 404).
  async function list(jobId) {
    if (!safeArtifactName(jobId)) return null;
    const dir = folder(jobId);
    if (!inside(dir)) return null;
    let names;
    try {
      names = await readdir(dir);
    } catch {
      return null;
    }
    const out = [];
    for (const name of names) {
      if (!safeArtifactName(name)) continue;
      try {
        const st = await stat(join(dir, name));
        if (st.isFile())
          out.push({ name, size: st.size, modified: iso(st.mtime) });
      } catch {
        // a file that went away between readdir and stat
      }
    }
    return { job: jobId, artifacts: sortNewest(out) };
  }

  async function count(jobId) {
    const l = await list(jobId);
    return l ? l.artifacts.length : 0;
  }

  // One file's bytes and mime, null when it is not there.
  async function file(jobId, name) {
    if (!safeArtifactName(jobId) || !safeArtifactName(name)) return null;
    const path = normalize(join(folder(jobId), name));
    if (!inside(path)) return null;
    try {
      const st = await stat(path);
      if (!st.isFile()) return null;
      return { bytes: await readFile(path), type: artifactMime(name) };
    } catch {
      return null;
    }
  }

  // The task detail's jobs with their counts: `artifacts` set only where
  // the folder has files, the way the hub leaves it absent for none.
  async function decorate(jobs) {
    return Promise.all(
      (jobs || []).map(async (j) => {
        const n = await count(j.id);
        return n ? { ...j, artifacts: n } : j;
      }),
    );
  }

  return { list, count, file, decorate };
}
