import { test } from "node:test";
import assert from "node:assert/strict";
import { count, compare } from "./clippy-baseline.mjs";

const msg = (lint, file, line, kind = ["lib"], col = 5) => JSON.stringify({
  reason: "compiler-message", target: { kind },
  message: { code: { code: lint }, message: `${lint} here`, spans: [{ is_primary: true, file_name: file, line_start: line, column_start: col }] },
});
const done = (ok = true) => JSON.stringify({ reason: "build-finished", success: ok });

test("counts per lint and file; the lib and its test build are one place", () => {
  const { counts, sites } = count([
    msg("clippy::unwrap_used", "a/src/x.rs", 3), msg("clippy::unwrap_used", "a/src/x.rs", 3),
    msg("clippy::unwrap_used", "a/src/x.rs", 9), msg("clippy::todo", "a/src/y.rs", 1), done(),
  ].join("\n"));
  assert.deepEqual(counts, { "clippy::unwrap_used": { "a/src/x.rs": 2 }, "clippy::todo": { "a/src/y.rs": 1 } });
  assert.deepEqual(sites["clippy::unwrap_used"]["a/src/x.rs"].map((s) => s.line), [3, 9]);
});

test("shipped-code lints are not counted in tests, examples and build scripts; others are", () => {
  const { counts } = count([
    msg("clippy::unwrap_used", "a/tests/t.rs", 1, ["test"]), msg("clippy::unwrap_used", "a/build.rs", 1, ["custom-build"]),
    msg("clippy::print_stdout", "a/examples/e.rs", 1, ["example"]), msg("clippy::todo", "a/tests/t.rs", 2, ["test"]), done(),
  ].join("\n"));
  assert.deepEqual(counts, { "clippy::todo": { "a/tests/t.rs": 1 } });
});

test("errors with a code, places outside the workspace, and other lines are not lints", () => {
  const { counts } = count([
    msg("E0308", "a/src/x.rs", 1), msg("clippy::todo", "/rustc/abc/library/core/src/x.rs", 1), msg("clippy::todo", "../dep/x.rs", 1),
    "   Compiling a v0.1.0", "{not json", done(),
  ].join("\n"));
  assert.deepEqual(counts, {});
});

test("no successful build-finished: no counts (a compile error must not read as zero warnings)", () => {
  assert.throws(() => count(msg("clippy::todo", "a/src/x.rs", 1)), /did not finish/);
  assert.throws(() => count([msg("clippy::todo", "a/src/x.rs", 1), done(false)].join("\n")), /did not finish/);
});

test("compare: above the baseline is up (a new file or lint too), below is down", () => {
  const base = { "clippy::unwrap_used": { "x.rs": 2, "y.rs": 1 } };
  const now = { "clippy::unwrap_used": { "x.rs": 3 }, "clippy::todo": { "z.rs": 1 } };
  assert.deepEqual(compare(base, now), {
    up: [{ lint: "clippy::todo", file: "z.rs", base: 0, now: 1 }, { lint: "clippy::unwrap_used", file: "x.rs", base: 2, now: 3 }],
    down: [{ lint: "clippy::unwrap_used", file: "y.rs", base: 1, now: 0 }],
  });
  assert.deepEqual(compare(base, base), { up: [], down: [] });
});
