#!/usr/bin/env node
// The lint gate for xplat/: clippy's warnings may not grow. Old warnings are frozen in xplat/clippy-baseline.json
// as counts per (lint, file); a count above the baseline fails and lists that lint's places in that file. Line numbers
// are not in the key, so moving code does not fail; a fix only lowers a count.
//
//   node scripts/clippy-baseline.mjs            check (exit 1: a count went up; exit 2: clippy did not run cleanly,
//                                               or this is not the baseline's platform or clippy)
//   node scripts/clippy-baseline.mjs --update   write the counts of now as the baseline (commit the file)
//   node scripts/clippy-baseline.mjs --json F   read clippy's JSON messages from F instead of running cargo
//
// It runs `cargo clippy --workspace --all-targets --locked --message-format=json -- --cap-lints warn`: an error lint
// (correctness, suspicious) is counted like a warning, so one does not stop the crates after it from being checked.
// unwrap_used, print_stdout, dbg_macro and disallowed_methods are for shipped code: not counted in test, example,
// bench and build-script targets. The counts depend on the platform (cfg) and on clippy's version: the baseline
// records both, and CI runs the same (macOS, the Rust in `clippy` below).
//
// Update the baseline when a count went down (this prints so), or with the reviewer's yes when a new warning is
// right (better: #[allow(clippy::…, reason = "…")] at the place, which needs no baseline change).
import { spawnSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { join, dirname, isAbsolute } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const xplat = join(dirname(fileURLToPath(import.meta.url)), "..", "xplat");
const BASELINE = join(xplat, "clippy-baseline.json");
export const SHIPPED_ONLY = new Set(["clippy::unwrap_used", "clippy::print_stdout", "clippy::dbg_macro", "clippy::disallowed_methods"]);
const NOT_SHIPPED = new Set(["test", "example", "bench", "custom-build"]);

/** Counts per lint and file, and each place, from cargo's JSON lines. Throws when the build did not finish. */
export function count(text) {
  const counts = {}, sites = {}, seen = new Set();
  let finished = null;
  for (const line of text.split("\n")) {
    if (!line.startsWith("{")) continue;
    let m;
    try { m = JSON.parse(line); } catch { continue; }
    if (m.reason === "build-finished") finished = m.success;
    if (m.reason !== "compiler-message") continue;
    const d = m.message, lint = d.code?.code;
    // A lint has a name; an error has a code like E0308 (and the build fails).
    if (!lint || /^E\d{4}$/.test(lint)) continue;
    if (SHIPPED_ONLY.has(lint) && m.target?.kind?.some((k) => NOT_SHIPPED.has(k))) continue;
    const sp = d.spans.find((s) => s.is_primary);
    // In a dependency or the standard library (a macro's inside): not ours.
    if (!sp || isAbsolute(sp.file_name) || sp.file_name.startsWith("..")) continue;
    const at = `${lint} ${sp.file_name}:${sp.line_start}:${sp.column_start}`;
    // The lib and its test build report the same place.
    if (seen.has(at)) continue;
    seen.add(at);
    const byFile = (counts[lint] ??= {});
    byFile[sp.file_name] = (byFile[sp.file_name] ?? 0) + 1;
    ((sites[lint] ??= {})[sp.file_name] ??= []).push({ line: sp.line_start, col: sp.column_start, text: d.message });
  }
  if (finished !== true) throw new Error("clippy did not finish (a compile error?): no counts");
  return { counts, sites };
}

/** The (lint, file) pairs above the baseline, and those below it. */
export function compare(base, now) {
  const up = [], down = [];
  const lints = new Set([...Object.keys(base), ...Object.keys(now)]);
  for (const lint of [...lints].sort()) {
    const files = new Set([...Object.keys(base[lint] ?? {}), ...Object.keys(now[lint] ?? {})]);
    for (const file of [...files].sort()) {
      const b = base[lint]?.[file] ?? 0, n = now[lint]?.[file] ?? 0;
      if (n > b) up.push({ lint, file, base: b, now: n });
      if (n < b) down.push({ lint, file, base: b, now: n });
    }
  }
  return { up, down };
}

const sorted = (o) => Object.fromEntries(Object.keys(o).sort().map((k) => [k, typeof o[k] === "object" ? sorted(o[k]) : o[k]]));
const total = (counts) => Object.values(counts).reduce((s, f) => s + Object.values(f).reduce((a, b) => a + b, 0), 0);

function clippyVersion() {
  const r = spawnSync("cargo", ["clippy", "--version"], { cwd: xplat, encoding: "utf8" });
  const v = r.stdout?.match(/clippy (\d+\.\d+\.\d+)/)?.[1];
  if (!v) throw new Error(`cargo clippy --version: ${r.stderr || r.stdout}`);
  return v;
}

function main(args) {
  const update = args.includes("--update");
  const jsonAt = args.indexOf("--json");
  const platform = process.platform;
  let text, clippy;
  if (jsonAt >= 0) {
    text = readFileSync(args[jsonAt + 1], "utf8");
    clippy = args.includes("--clippy") ? args[args.indexOf("--clippy") + 1] : null;
  } else {
    clippy = clippyVersion();
    const r = spawnSync("cargo", ["clippy", "--workspace", "--all-targets", "--locked", "--message-format=json", "--", "--cap-lints", "warn"],
      { cwd: xplat, encoding: "utf8", maxBuffer: 1 << 30, stdio: ["ignore", "pipe", "inherit"] });
    if (r.status !== 0) {
      console.error(`clippy-baseline: cargo clippy exited ${r.status ?? r.signal}: fix the build first`);
      return 2;
    }
    text = r.stdout;
  }
  let now;
  try { ({ counts: now } = count(text)); } catch (e) { console.error(`clippy-baseline: ${e.message}`); return 2; }
  const sites = count(text).sites;

  if (update) {
    const out = {
      about: "Frozen clippy counts per lint and file: scripts/clippy-baseline.mjs fails when one goes up. Update with node scripts/clippy-baseline.mjs --update on this platform and clippy.",
      platform, clippy, total: total(now), counts: sorted(now),
    };
    writeFileSync(BASELINE, JSON.stringify(out, null, 2) + "\n");
    console.log(`clippy-baseline: wrote ${out.total} warnings (${platform}, clippy ${clippy}) to xplat/clippy-baseline.json`);
    return 0;
  }

  const base = JSON.parse(readFileSync(BASELINE, "utf8"));
  if (base.platform !== platform || (clippy && base.clippy !== clippy)) {
    console.error(`clippy-baseline: the baseline is for ${base.platform}, clippy ${base.clippy}; this is ${platform}, clippy ${clippy}. ` +
      `The counts differ by platform and clippy version: run it there (CI: xplat / Clippy), or update the baseline and the CI's Rust together.`);
    return 2;
  }
  const { up, down } = compare(base.counts, now);
  const gh = process.env.GITHUB_ACTIONS === "true";
  for (const u of up) {
    console.error(`NEW  ${u.lint} in ${u.file}: ${u.base} in the baseline, ${u.now} now. Its places (one or more of them is new):`);
    for (const s of sites[u.lint]?.[u.file] ?? []) {
      console.error(`       ${u.file}:${s.line}:${s.col}  ${s.text}`);
      if (gh) console.log(`::error file=xplat/${u.file},line=${s.line},col=${s.col},title=${u.lint}::${s.text}`);
    }
  }
  for (const d of down) console.log(`lower ${d.lint} in ${d.file}: ${d.base} → ${d.now}`);
  console.log(`clippy-baseline: ${total(now)} warnings now, ${base.total} in the baseline (${platform}, clippy ${clippy ?? "?"})`);
  if (up.length) {
    console.error(`clippy-baseline: ${up.length} (lint, file) counts went up. Fix the new warning, or allow it at the place with a reason ` +
      `(#[allow(clippy::…, reason = "…")]). Do not raise the baseline for new code.`);
    return 1;
  }
  if (down.length) console.log("clippy-baseline: counts went down: run node scripts/clippy-baseline.mjs --update and commit xplat/clippy-baseline.json");
  return 0;
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) process.exit(main(process.argv.slice(2)));
