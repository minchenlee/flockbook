#!/usr/bin/env node
// The release version a helper build bakes in (FLOCKBOOK_VERSION, read by update-check), from the git ref the
// build checks out, and the check that every built helper says it. helpers.yml uses both; a release's ref is
// "vX.Y.Z" (a dispatch, or release.yml's ref_name) or "refs/tags/vX.Y.Z" (github.ref on a tag push).
//
//   node scripts/helper-release.mjs version <ref>             x.y.z, or nothing when the ref is no release
//   node scripts/helper-release.mjs check <ref> <version.txt>...
//       each file is a helper's `version` output; release: its "release" is x.y.z, else null. Exit 1 when not.
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

export function releaseOf(ref) {
  const m = /^(?:refs\/tags\/)?v(\d+\.\d+\.\d+)$/.exec(String(ref ?? "").trim());
  return m ? m[1] : null;
}

// The "release" a helper's `version` output names (its `version` line), or undefined when it has none.
export function releaseIn(text) {
  for (const line of String(text).split("\n")) {
    let o; try { o = JSON.parse(line); } catch { continue; }
    if (o?.type === "version") return "release" in o ? o.release : undefined;
  }
  return undefined;
}

// Problems, one per helper that does not say the version this ref should bake in.
export function check(ref, files) {
  const want = releaseOf(ref);
  const bad = [];
  for (const [name, text] of files) {
    const got = releaseIn(text);
    if (got === undefined) bad.push(`${name}: no "release" in its version output`);
    else if (got !== want) bad.push(`${name}: release ${JSON.stringify(got)}, want ${JSON.stringify(want)} (ref ${JSON.stringify(ref)})`);
  }
  return bad;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const [what, ref, ...paths] = process.argv.slice(2);
  if (what === "version" && ref !== undefined && !paths.length) {
    const v = releaseOf(ref);
    if (v) process.stdout.write(`${v}\n`);
  } else if (what === "check" && ref !== undefined && paths.length) {
    const bad = check(ref, paths.map((p) => [p, readFileSync(p, "utf8")]));
    for (const b of bad) console.error(b);
    if (bad.length) process.exit(1);
    console.log(`${paths.length} helpers: release ${JSON.stringify(releaseOf(ref))}`);
  } else {
    console.error("usage: helper-release.mjs version <ref> | check <ref> <version.txt>...");
    process.exit(2);
  }
}
