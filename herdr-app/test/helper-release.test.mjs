// scripts/helper-release.mjs: the version a release's helpers bake in (update-check), from the ref the build
// checks out, and the package job's check that each built helper says it.
import { test } from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { writeFileSync, mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { releaseOf, releaseIn, check } from "../../scripts/helper-release.mjs";

const script = join(dirname(fileURLToPath(import.meta.url)), "..", "..", "scripts", "helper-release.mjs");
const out = (release) => [JSON.stringify({ type: "header", protocol: 1, sidecar: "0.1.0", cmd: "version" }),
  JSON.stringify({ type: "version", commands: ["version"], ...(release === undefined ? {} : { release }) })].join("\n");

test("a tag push (refs/tags/vX.Y.Z) is a release as much as a dispatch (vX.Y.Z)", () => {
  assert.equal(releaseOf("refs/tags/v0.9.0"), "0.9.0", "github.ref on a tag push: the case that baked nothing in");
  assert.equal(releaseOf("v0.9.0"), "0.9.0");
  assert.equal(releaseOf("v0.10.12"), "0.10.12");
  for (const r of ["", null, undefined, "refs/heads/ci/mod-update", "ci/mod-update", "main", "v1.0.0-rc1", "refs/tags/v1.0", "0.9.0", "refs/tags/x0.9.0"]) {
    assert.equal(releaseOf(r), null, String(r));
  }
});

test("the package check: every helper names the tag's version, and a ci build names none", () => {
  assert.equal(releaseIn(out("0.9.0")), "0.9.0");
  assert.equal(releaseIn(out(null)), null);
  assert.equal(releaseIn(out(undefined)), undefined);
  assert.deepEqual(check("refs/tags/v0.9.0", [["a", out("0.9.0")], ["b", out("0.9.0")]]), []);
  assert.deepEqual(check("refs/heads/ci/x", [["a", out(null)]]), []);
  assert.equal(check("refs/tags/v0.9.0", [["a", out("0.9.0")], ["b", out(null)]]).length, 1, "one helper built without it");
  assert.equal(check("v0.9.0", [["a", out("0.8.0")]]).length, 1);
  assert.equal(check("refs/heads/ci/x", [["a", out("0.9.0")]]).length, 1, "a ci build must not claim a release");
  assert.equal(check("v0.9.0", [["a", out(undefined)]]).length, 1, "an old helper that does not say");
});

test("the command line", () => {
  const run = (...a) => spawnSync(process.execPath, [script, ...a], { encoding: "utf8" });
  assert.equal(run("version", "refs/tags/v0.9.0").stdout, "0.9.0\n");
  assert.equal(run("version", "refs/heads/main").stdout, "");
  const dir = mkdtempSync(join(tmpdir(), "fb-rel-"));
  writeFileSync(join(dir, "good.txt"), out("0.9.0"));
  writeFileSync(join(dir, "none.txt"), out(null));
  assert.equal(run("check", "refs/tags/v0.9.0", join(dir, "good.txt")).status, 0);
  assert.equal(run("check", "refs/tags/v0.9.0", join(dir, "good.txt"), join(dir, "none.txt")).status, 1);
  assert.equal(run("bogus").status, 2);
});
