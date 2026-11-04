// install.sh, run against temp homes only. HOME is a temp folder too, so the script's default
// (~/.config/herdr-app) can never be the real one. A fake herdr-app on PATH records its calls.
import { test } from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, existsSync, chmodSync, readdirSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const modDir = join(dirname(fileURLToPath(import.meta.url)), "..");
const script = join(modDir, "install.sh");
const ours = JSON.parse(readFileSync(join(modDir, "keys.json"), "utf8"));

function sandbox() {
  const root = mkdtempSync(join(tmpdir(), "fb-install-"));
  const home = join(root, "user"), app = join(root, "app"), bin = join(root, "bin");
  mkdirSync(home); mkdirSync(app); mkdirSync(bin);
  const fake = join(bin, "herdr-app");
  writeFileSync(fake, `#!/bin/sh\necho "$HERDR_APP_HOME $*" >> "${join(root, "calls")}"\n[ -f "${join(root, "fail")}" ] && { echo "bad mod"; exit 1; }\necho ok\n`);
  chmodSync(fake, 0o755);
  const helper = join(root, "flockbook-modfeed");
  writeFileSync(helper, "#!/bin/sh\necho helper\n"); chmodSync(helper, 0o755);
  const run = (...args) => spawnSync("/bin/sh", [script, ...args], {
    encoding: "utf8", env: { PATH: `${bin}:/usr/bin:/bin`, HOME: home },
  });
  const calls = () => existsSync(join(root, "calls")) ? readFileSync(join(root, "calls"), "utf8").trim().split("\n") : [];
  return { root, home, app, helper, run, calls };
}
const read = (p) => readFileSync(p, "utf8");
const keysOf = (app) => JSON.parse(read(join(app, "keys.json")));

test("installs into an empty home, puts the helper in place and runs herdr-app check", () => {
  const s = sandbox();
  const r = s.run("--home", s.app, "--helper", s.helper);
  assert.equal(r.status, 0, r.stdout + r.stderr);
  assert.ok(existsSync(join(s.app, "layouts/flockbook.json")));
  assert.ok(existsSync(join(s.app, "mods/flockbook/mod.json")));
  assert.ok(existsSync(join(s.app, "themes/flockbook-light.json")));
  assert.equal(read(join(s.app, "flockbook/bin/flockbook-modfeed")), read(s.helper));
  assert.deepEqual(keysOf(s.app), ours);
  assert.deepEqual(s.calls(), [`${s.app} check ${s.app}/mods/flockbook`]);
});

test("--dry-run writes nothing and does not run herdr-app", () => {
  const s = sandbox();
  const r = s.run("--home", s.app, "--helper", s.helper, "--dry-run");
  assert.equal(r.status, 0, r.stdout + r.stderr);
  assert.deepEqual(readdirSync(s.app), []);
  assert.deepEqual(s.calls(), []);
  assert.match(r.stdout, /layouts\/flockbook\.json/);
  assert.match(r.stdout, /dry run/i);
});

test("stops without writing when the person has their own layout called flockbook", () => {
  const s = sandbox();
  mkdirSync(join(s.app, "layouts"));
  writeFileSync(join(s.app, "layouts/flockbook.json"), "{\"mine\": true}\n");
  const r = s.run("--home", s.app);
  assert.equal(r.status, 1);
  assert.match(r.stderr, /layouts\/flockbook\.json/);
  assert.equal(read(join(s.app, "layouts/flockbook.json")), "{\"mine\": true}\n");
  assert.ok(!existsSync(join(s.app, "mods")));
  assert.ok(!existsSync(join(s.app, "keys.json")));
});

test("stops when the person has their own mod folder called flockbook", () => {
  const s = sandbox();
  mkdirSync(join(s.app, "mods/flockbook"), { recursive: true });
  writeFileSync(join(s.app, "mods/flockbook/main.js"), "mine\n");
  const r = s.run("--home", s.app);
  assert.equal(r.status, 1);
  assert.match(r.stderr, /mods\/flockbook/);
  assert.equal(read(join(s.app, "mods/flockbook/main.js")), "mine\n");
  assert.ok(!existsSync(join(s.app, "layouts")));
});

test("merges keys: keeps the person's sections and keys, never replaces a key they use", () => {
  const s = sandbox();
  writeFileSync(join(s.app, "keys.json"), JSON.stringify({
    global: { "cmd-k": "app.palette" },
    "layout:flockbook": { "cmd-b": "app.something-else" },
  }));
  const r = s.run("--home", s.app);
  assert.equal(r.status, 0, r.stdout + r.stderr);
  const k = keysOf(s.app);
  assert.deepEqual(k.global, { "cmd-k": "app.palette" });
  assert.equal(k["layout:flockbook"]["cmd-b"], "app.something-else");
  assert.equal(k["layout:flockbook"]["cmd-t"], "tab.new");
  assert.match(r.stdout, /cmd-b/);
});

test("installing again updates its own files, keeps edits the person made", () => {
  const s = sandbox();
  assert.equal(s.run("--home", s.app).status, 0);
  writeFileSync(join(s.app, "mods/flockbook/main.js"), "stale copy from an older release\n");
  // An older release's file the person never touched: the marker still matches, so it is ours.
  const marker = JSON.parse(read(join(s.app, "flockbook/install.json")));
  marker.files["mods/flockbook/main.js"] = sha("stale copy from an older release\n");
  writeFileSync(join(s.app, "flockbook/install.json"), JSON.stringify(marker));
  const again = s.run("--home", s.app);
  assert.equal(again.status, 0, again.stdout + again.stderr);
  assert.equal(read(join(s.app, "mods/flockbook/main.js")), read(join(modDir, "mods/flockbook/main.js")));

  writeFileSync(join(s.app, "layouts/flockbook.json"), "{\"edited\": true}\n");
  const third = s.run("--home", s.app);
  assert.equal(third.status, 1);
  assert.match(third.stderr, /layouts\/flockbook\.json/);
  assert.equal(read(join(s.app, "layouts/flockbook.json")), "{\"edited\": true}\n");
});

test("installing again removes its own files a newer version dropped, nothing else", () => {
  const s = sandbox();
  assert.equal(s.run("--home", s.app).status, 0);
  writeFileSync(join(s.app, "mods/flockbook/old.js"), "gone\n");
  writeFileSync(join(s.app, "notes.txt"), "mine\n");
  const marker = JSON.parse(read(join(s.app, "flockbook/install.json")));
  marker.files["mods/flockbook/old.js"] = sha("gone\n");
  marker.files["notes.txt"] = sha("mine\n");
  writeFileSync(join(s.app, "flockbook/install.json"), JSON.stringify(marker));
  assert.equal(s.run("--home", s.app).status, 0);
  assert.ok(!existsSync(join(s.app, "mods/flockbook/old.js")));
  assert.equal(read(join(s.app, "notes.txt")), "mine\n");
});

test("files herdr-app writes into the mod folder do not block a second install", () => {
  const s = sandbox();
  assert.equal(s.run("--home", s.app).status, 0);
  writeFileSync(join(s.app, "mods/flockbook/jsconfig.json"), "{}\n");
  writeFileSync(join(s.app, "mods/flockbook/gpui-kit.d.ts"), "// types\n");
  const r = s.run("--home", s.app);
  assert.equal(r.status, 0, r.stdout + r.stderr);
  assert.equal(read(join(s.app, "mods/flockbook/jsconfig.json")), "{}\n");
});

test("a failed herdr-app check fails the install and says so", () => {
  const s = sandbox();
  writeFileSync(join(s.root, "fail"), "");
  const r = s.run("--home", s.app);
  assert.equal(r.status, 1);
  assert.match(r.stdout + r.stderr, /herdr-app check/);
});

test("without --helper and with no helper in place, it says where to get one", () => {
  const s = sandbox();
  const r = s.run("--home", s.app);
  assert.equal(r.status, 0, r.stdout + r.stderr);
  assert.match(r.stdout, /github\.com\/minchenlee\/flockbook\/releases/);
  assert.match(r.stdout, /swift build -c release --product flockbook-modfeed/);
});

test("the home defaults to HERDR_APP_HOME, then ~/.config/herdr-app", () => {
  const s = sandbox();
  const r = spawnSync("/bin/sh", [script, "--dry-run"], { encoding: "utf8",
    env: { PATH: `/usr/bin:/bin`, HOME: s.home } });
  assert.equal(r.status, 0, r.stdout + r.stderr);
  assert.ok(r.stdout.includes(join(s.home, ".config/herdr-app")), r.stdout);
  const e = spawnSync("/bin/sh", [script, "--dry-run"], { encoding: "utf8",
    env: { PATH: `/usr/bin:/bin`, HOME: s.home, HERDR_APP_HOME: s.app } });
  assert.ok(e.stdout.includes(s.app), e.stdout);
});

import { createHash } from "node:crypto";
function sha(text) { return createHash("sha256").update(text).digest("hex"); }
