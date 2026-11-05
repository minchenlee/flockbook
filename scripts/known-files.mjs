#!/usr/bin/env node
// The known-files table: the sha256 of every file `flockbook-modfeed install` puts in a herdr-app home
// (layouts/flockbook.json, themes/*.json, mods/flockbook/**), in every version that left this repo: each
// release from v0.7.0, and main between releases (the Herdr layout gallery copies main). The install treats a
// file with one of these hashes as an older Flockbook: it replaces it, or removes it when this version no longer
// has it. A file whose hash is not here is the person's, and the install stops.
//
// It is not checked in: xplat/flockbook-modfeed/build.rs runs this when it builds the helper and embeds the
// table. It needs git history and the tags (CI: fetch-depth 0).
//   node scripts/known-files.mjs [--out <file>]   (stdout without --out)
//
// A line: `<sha256>  <version>  <path>`. The version is the earliest release tag whose tree has the file, else
// `<tag>+main` (main after that release).
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync, readdirSync, statSync } from "node:fs";
import { join, dirname, relative, sep } from "node:path";
import { fileURLToPath } from "node:url";

const repo = join(dirname(fileURLToPath(import.meta.url)), "..");
const FIRST = "v0.7.0";
const FOLDERS = ["herdr-app", "mod", "herdr-mod"]; // the mod's folder, newest name first
const HEAD = `# The files of every Flockbook for the Herdr app that left this repo: sha256, version, path in the home.
# Made by scripts/known-files.mjs when the helper is built; flockbook-modfeed install embeds it.
`;

const sha = (b) => createHash("sha256").update(b).digest("hex");
const git = (args, opts = {}) => execFileSync("git", args, { cwd: repo, maxBuffer: 1 << 28, ...opts });
const isRelease = (t) => /^v\d+\.\d+\.\d+$/.test(t);
const cmpVer = (a, b) => { const x = a.slice(1).split(".").map(Number), y = b.slice(1).split(".").map(Number); return x[0] - y[0] || x[1] - y[1] || x[2] - y[2]; };

// The same choice as xplat/flockbook-modfeed/build.rs: what the install puts in a home.
export function installed(rel) {
  if (rel === "layouts/flockbook.json") return true;
  if (/^themes\/[^/]+\.json$/.test(rel)) return true;
  if (!rel.startsWith("mods/flockbook/")) return false;
  const name = rel.split("/").pop(), top = rel.split("/").length === 3;
  if (name === ".DS_Store") return false;
  return !(top && (name === "jsconfig.json" || name.endsWith(".d.ts")));
}

export function readTable(text) {
  const t = new Map(); // "path\tsha" -> version
  for (const line of text.split("\n")) {
    if (!line.trim() || line.startsWith("#")) continue;
    const [h, v, p] = line.trim().split(/\s+/);
    t.set(`${p}\t${h}`, v);
  }
  return t;
}

// Earlier releases first, a release before a `+main` label, and of two `+main`, the earlier one.
function better(a, b) {
  if (!a) return b;
  const ra = isRelease(a), rb = isRelease(b);
  if (ra !== rb) return ra ? a : b;
  return cmpVer(a.replace(/\+main$/, ""), b.replace(/\+main$/, "")) <= 0 ? a : b;
}

export function writeTable(t) {
  const rows = [...t].map(([k, v]) => { const [p, h] = k.split("\t"); return { p, h, v }; })
    .sort((a, b) => (a.p < b.p ? -1 : a.p > b.p ? 1 : a.h < b.h ? -1 : 1));
  return HEAD + rows.map((r) => `${r.h}  ${r.v}  ${r.p}`).join("\n") + "\n";
}

// The files in the working tree, as path in the home -> bytes.
function treeNow() {
  const base = join(repo, "herdr-app"), out = new Map();
  const walk = (d) => {
    for (const n of readdirSync(d).sort()) {
      const p = join(d, n);
      if (statSync(p).isDirectory()) walk(p);
      else { const rel = relative(base, p).split(sep).join("/"); if (installed(rel)) out.set(rel, readFileSync(p)); }
    }
  };
  walk(join(base, "layouts")); walk(join(base, "themes")); walk(join(base, "mods", "flockbook"));
  // The layouts folder may hold other people's layouts in a dev checkout: only ours counts.
  for (const k of [...out.keys()]) if (k.startsWith("layouts/") && k !== "layouts/flockbook.json") out.delete(k);
  return out;
}

// The files at a commit, as path in the home -> sha256. A blob is read once, however many commits have it.
const blobSums = new Map();
function treeAt(commit) {
  const rows = git(["ls-tree", "-r", commit]).toString().split("\n").filter(Boolean)
    .map((l) => { const [meta, name] = l.split("\t"); return { blob: meta.split(" ")[2], name }; });
  const folder = FOLDERS.find((f) => rows.some((r) => r.name === `${f}/layouts/flockbook.json`));
  if (!folder) return new Map();
  const out = new Map();
  for (const { blob, name } of rows) {
    if (!name.startsWith(folder + "/")) continue;
    const rel = name.slice(folder.length + 1);
    if (!installed(rel)) continue;
    if (!blobSums.has(blob)) blobSums.set(blob, sha(git(["cat-file", "blob", blob])));
    out.set(rel, blobSums.get(blob));
  }
  return out;
}

function labelNow() {
  try {
    const at = git(["tag", "--points-at", "HEAD"]).toString().split("\n").filter(isRelease).sort(cmpVer);
    if (at.length) return at[0];
    return git(["describe", "--tags", "--abbrev=0", "--match", "v[0-9]*"]).toString().trim() + "+main";
  } catch { return "v0.0.0+main"; }
}

// The table, from git: each commit on HEAD's first-parent line from v0.7.0 (main, and this branch on top of it),
// each release tag from v0.7.0, and the working tree as it is now.
export function generate() {
  const t = new Map();
  const tagsAt = new Map();
  for (const tag of git(["tag", "--list", "v*"]).toString().split("\n").filter(isRelease)) {
    const c = git(["rev-list", "-n1", tag]).toString().trim();
    if (!tagsAt.has(c)) tagsAt.set(c, []);
    tagsAt.get(c).push(tag);
  }
  let start;
  try { start = git(["rev-list", "-n1", FIRST], { stdio: ["ignore", "pipe", "ignore"] }).toString().trim(); }
  catch { throw new Error(`no ${FIRST} tag here: check out with the history and the tags (fetch-depth: 0)`); }
  const line = git(["rev-list", "--first-parent", "--reverse", "HEAD"]).toString().trim().split("\n");
  const from = line.indexOf(start);
  if (from < 0) throw new Error(`${FIRST} is not on HEAD's first-parent line (a shallow checkout?)`);
  let last = FIRST;
  for (const c of line.slice(from)) {
    const tags = (tagsAt.get(c) ?? []).sort(cmpVer);
    if (tags.length) last = tags[tags.length - 1];
    const label = tags.length ? tags[0] : `${last}+main`;
    for (const [p, h] of treeAt(c)) t.set(`${p}\t${h}`, better(t.get(`${p}\t${h}`), label));
  }
  // Release trees that are not on this line (a tag on a branch) count too.
  for (const [c, tags] of tagsAt) {
    const tag = tags.sort(cmpVer)[0];
    if (cmpVer(tag, FIRST) < 0) continue;
    for (const [p, h] of treeAt(c)) t.set(`${p}\t${h}`, better(t.get(`${p}\t${h}`), tag));
  }
  for (const [p, b] of treeNow()) { const k = `${p}\t${sha(b)}`; t.set(k, better(t.get(k), `${last}+main`)); }
  return t;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const args = process.argv.slice(2);
  const i = args.indexOf("--out");
  if (args.length && (i !== 0 || args.length !== 2)) { console.error("usage: node scripts/known-files.mjs [--out <file>]"); process.exit(2); }
  let t;
  try { t = generate(); } catch (e) { console.error(`known-files: ${e.message}`); process.exit(1); }
  const text = writeTable(t);
  if (i === 0) { writeFileSync(args[1], text); console.error(`known-files: ${t.size} lines in ${args[1]}`); }
  else process.stdout.write(text);
}
