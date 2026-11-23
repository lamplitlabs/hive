#!/usr/bin/env node
// The diagrams drawn in text in the repository's docs: lists each one and
// says what is badly drawn (a column that drifted, an open box, a line too
// wide, a character the fonts do not hold). src/lib/diagram.ts has the rules,
// docs/development.md the way to write one.
//
//   node scripts/check-diagrams.mjs                 every page of docs/
//   node scripts/check-diagrams.mjs landing.md …    only these (relative to docs/)
//   node scripts/check-diagrams.mjs --file F.md …   any Markdown file: a decision
//                                                    record written to a file before
//                                                    `hive record new adr --body-file`
//
// Exit 0 when every diagram is well drawn, 1 when one is not, 2 for a file
// that is not there. `npm test` checks the pages of docs/ through
// src/lib/docs.test.mjs; this prints where each diagram is, for the writer.
// It needs Node 22.18 or later and nothing else: no `npm ci`, so a fresh
// clone can run it.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const { findDiagrams } = await import("../src/lib/diagram.ts");

// Run from anywhere: the docs are the ones of the checkout this script is in.
const docs = process.env.HIVE_DOCS_DIR ? path.resolve(process.env.HIVE_DOCS_DIR) : path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../../docs");

const args = process.argv.slice(2);
const loose = args[0] === "--file";
const wanted = (loose ? args.slice(1) : args).map((f) => (loose ? f : f.replace(/^(\.\/)?docs\//, "")));

function pages(dir, prefix = "") {
  return fs
    .readdirSync(dir, { withFileTypes: true })
    .flatMap((e) => (e.isDirectory() ? pages(path.join(dir, e.name), `${prefix}${e.name}/`) : /\.md$/i.test(e.name) ? [`${prefix}${e.name}`] : []))
    .sort();
}

/** Each file to report on: its name as the writer knows it, and where it is. */
let files;
if (loose) {
  const missing = wanted.filter((f) => !fs.existsSync(f));
  if (!wanted.length || missing.length) {
    console.error(wanted.length ? `no such file: ${missing.join(", ")}` : "--file needs at least one Markdown file");
    process.exit(2);
  }
  files = wanted.map((f) => ({ name: f, at: f }));
} else {
  const all = pages(docs);
  const unknown = wanted.filter((f) => !all.includes(f));
  if (unknown.length) {
    console.error(`not a page of docs/: ${unknown.join(", ")}`);
    process.exit(2);
  }
  files = all.filter((f) => !wanted.length || wanted.includes(f)).map((f) => ({ name: `docs/${f}`, at: path.join(docs, f) }));
}

let count = 0;
let bad = 0;
for (const file of files) {
  const diagrams = findDiagrams(fs.readFileSync(file.at, "utf8"));
  if (!diagrams.length) {
    if (wanted.length) console.log(`${file.name}: no diagram`);
    continue;
  }
  for (const d of diagrams) {
    count += 1;
    const where = `${file.name}:${d.line}`;
    if (!d.problems.length) {
      console.log(`${where}: ${d.rows} rows, ${d.columns} columns, well drawn (${d.words.slice(0, 4).join(", ")}${d.words.length > 4 ? ", …" : ""})`);
      continue;
    }
    bad += 1;
    console.log(`${where}: ${d.problems.length} problem(s)`);
    for (const p of d.problems) console.log(`  ${p}`);
  }
}
console.log(bad ? `${bad} of ${count} diagram(s) badly drawn.` : `${count} diagram(s), all well drawn.`);
process.exit(bad ? 1 : 0);
