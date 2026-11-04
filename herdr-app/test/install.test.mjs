// install.sh is a thin wrapper around `flockbook-modfeed install` (its behaviour is tested in Rust:
// xplat/flockbook-modfeed/tests/install.rs). These run only with a Rust helper in FLOCKBOOK_MODFEED,
// against temp homes; HOME is a temp folder too.
import { test } from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, mkdirSync, writeFileSync, readdirSync, existsSync, copyFileSync, chmodSync, readFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const modDir = join(dirname(fileURLToPath(import.meta.url)), "..");
const helper = process.env.FLOCKBOOK_MODFEED;
const skip = helper && existsSync(helper) ? false : "needs FLOCKBOOK_MODFEED (a Rust flockbook-modfeed)";

function sandbox() {
  const root = mkdtempSync(join(tmpdir(), "fb-install-"));
  const home = join(root, "user"), app = join(root, "app");
  mkdirSync(home); mkdirSync(app);
  return { root, home, app };
}
const run = (script, args, env) => spawnSync("/bin/sh", [script, ...args], { encoding: "utf8", env: { PATH: "/usr/bin:/bin", ...env } });

test("without a helper it stops and says how to build one", () => {
  const s = sandbox();
  const r = run(join(modDir, "install.sh"), ["--home", s.app], { HOME: s.home });
  assert.equal(r.status, 1);
  assert.match(r.stderr, /cargo build --release -p flockbook-modfeed/);
  assert.deepEqual(readdirSync(s.app), []);
});

test("--dry-run goes to flockbook-modfeed install and writes nothing", { skip }, () => {
  const s = sandbox();
  const r = run(join(modDir, "install.sh"), ["--home", s.app, "--dry-run"], { HOME: s.home, FLOCKBOOK_MODFEED: helper });
  assert.equal(r.status, 0, r.stdout + r.stderr);
  assert.match(r.stdout, /Dry run: would install into/);
  assert.match(r.stdout, /flockbook\/bin\/flockbook-modfeed/);
  assert.deepEqual(readdirSync(s.app), []);
});

test("as in the zip: bin/ is the helper and remote/ goes to <home>/flockbook/remote", { skip }, () => {
  const s = sandbox();
  const zip = join(s.root, "zip");
  mkdirSync(join(zip, "bin"), { recursive: true });
  copyFileSync(join(modDir, "install.sh"), join(zip, "install.sh"));
  copyFileSync(helper, join(zip, "bin/flockbook-modfeed")); chmodSync(join(zip, "bin/flockbook-modfeed"), 0o755);
  const t = "aarch64-unknown-linux-musl", body = "#!/bin/sh\necho remote\n";
  mkdirSync(join(zip, "remote", t), { recursive: true });
  writeFileSync(join(zip, "remote", t, "flockbook-modfeed"), body);
  writeFileSync(join(zip, "remote/SHA256SUMS"), `${createHash("sha256").update(body).digest("hex")}  ${t}/flockbook-modfeed\n`);
  const r = run(join(zip, "install.sh"), ["--home", s.app], { HOME: s.home });
  assert.equal(r.status, 0, r.stdout + r.stderr);
  assert.ok(existsSync(join(s.app, "mods/flockbook/mod.json")));
  assert.equal(readFileSync(join(s.app, "flockbook/bin/flockbook-modfeed")).length, readFileSync(helper).length);
  assert.equal(readFileSync(join(s.app, "flockbook/remote", t, "flockbook-modfeed"), "utf8"), body);
});
