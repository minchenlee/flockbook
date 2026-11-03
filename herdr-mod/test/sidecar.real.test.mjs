// The mod's sidecar calls against a real sidecar binary: set FLOCKBOOK_MODFEED to its path (CI does, for the Rust
// one in rust/). Without it these tests are skipped. herdr-core is a fake on a Unix socket (no app, no network).
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { createServer } from "node:net";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fake, reset } from "./stub/herdr.mjs";
import { call, feed, card, approve, PROTOCOL } from "../mods/flockbook/sidecar.js";

const bin = process.env.FLOCKBOOK_MODFEED;
const skip = !bin || process.platform === "win32" ? "set FLOCKBOOK_MODFEED to a sidecar binary (Unix)" : false;

const screen = "Bash command\n  rm -rf build\nDo you want to proceed?\n❯ 1. Yes\n  2. No";
const terminals = [{ address: "t1", argv: ["claude"], pid: 4242, created_ms: 1, cwd: "/tmp/proj", state: "live",
  status: { state: "blocked", kind: "permission" }, foreground: null }];
let dir, sock, server, keys = [], env = {};

before(async () => {
  if (skip) return;
  dir = mkdtempSync(join(tmpdir(), "fb-sidecar-"));
  sock = join(dir, "hc.sock");
  server = createServer((c) => {
    let buf = "";
    c.on("data", (d) => {
      buf += d;
      let nl;
      while ((nl = buf.indexOf("\n")) >= 0) {
        const req = JSON.parse(buf.slice(0, nl)); buf = buf.slice(nl + 1);
        const ok = (result) => c.write(JSON.stringify({ jsonrpc: "2.0", id: req.id, result }) + "\n");
        if (req.method === "terminal.list") ok({ terminals });
        else if (req.method === "terminal.read") ok({ text: screen });
        else if (req.method === "terminal.keys") { keys.push(req.params.keys); ok({}); }
        else c.write(JSON.stringify({ jsonrpc: "2.0", id: req.id, error: { code: -32601, message: "no" } }) + "\n");
      }
    });
  });
  await new Promise((r) => server.listen(sock, r));
});
const appHome = fake.appHome;
after(() => { fake.appHome = appHome; if (server) server.close(); if (dir) rmSync(dir, { recursive: true, force: true }); });

// herdr's exec, run for real: the mod's own path is swapped for the binary under test.
// The cache file goes into the temp folder, not the stub's app home.
function useBinary(extraEnv = {}) {
  reset();
  fake.appHome = dir;
  fake.exec = (_cmd, args) => new Promise((resolve) => {
    execFile(bin, args, { env: { PATH: process.env.PATH, HOME: dir, HC_SOCKET: sock, ...extraEnv } }, (err, stdout, stderr) =>
      resolve({ code: err ? (err.code ?? 1) : 0, stdout, stderr }));
  });
}

test("real sidecar: version speaks this mod's protocol", { skip }, async () => {
  useBinary();
  const a = await call("version");
  assert.equal(a.error, null);
  assert.equal(a.header.protocol, PROTOCOL);
  assert.equal(a.end.ok, true);
  for (const c of ["feed", "status", "card", "approve", "answer", "send"]) assert.ok(a.lines[0].commands.includes(c), c);
});

test("real sidecar: a known failure is an error line, then end", { skip }, async () => {
  useBinary({ HC_SOCKET: join(dir, "missing.sock") });
  const a = await feed("t1");
  assert.equal(a.error?.code, "no-core");
  assert.equal(a.end?.ok, false);
});

test("real sidecar: card reads, approve sends only for that request", { skip }, async () => {
  useBinary();
  const c = await card("t1");
  assert.equal(c.error, null);
  const cardLine = c.lines.find((l) => l.type === "card");
  assert.equal(cardLine.kind, "approval");
  assert.equal(cardLine.command, "rm -rf build");
  assert.match(cardLine.request, /^screen-/);
  assert.deepEqual(keys, []);
  const wrong = await approve("t1", "screen-other", true);
  assert.equal(wrong.lines.find((l) => l.type === "done").result, "changed");
  assert.deepEqual(keys, []);
  const ok = await approve("t1", cardLine.request, true);
  assert.equal(ok.lines.find((l) => l.type === "done").result, "sent");
  assert.deepEqual(keys, [["1"]]);
});
