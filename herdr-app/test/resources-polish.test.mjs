// Resources polish (hg-res-alive-shared, hg-res-panel-follow hints): the open check without a heartbeat, the hint for an
// old remote helper, and Show in chat for a row the chat has not loaded.
import { test, beforeEach, afterEach } from "node:test";
import assert from "node:assert/strict";
import { fake, log, reset } from "./stub/herdr.mjs";
import { textOf, byId, calls } from "./stub/ui.mjs";
import { fakeCx } from "./fake-cx.mjs";
import { agent, space } from "./fixtures.mjs";
import * as R from "../mods/flockbook/resources.js";
import { forgetCommands } from "../mods/flockbook/sidecar.js";
import { command } from "../mods/flockbook/top.js";
import { ChatView } from "../mods/flockbook/chat.js";

const jl = (...lines) => ({ code: 0, stdout: lines.map((l) => JSON.stringify(l)).join("\n") + "\n", stderr: "" });
const header = (cmd) => ({ type: "header", protocol: 1, sidecar: "0.1.0", cmd });
const end = (cursor = "c1", full = true) => ({ type: "end", ok: true, ms: 1, cursor, full });
const res = (id, last_ts) => ({ type: "res", id, kind: "link", target: `https://x.com/${id}`, title: id, detail: "x.com", row: `r-${id}`, first_ts: last_ts, last_ts, mentions: 1 });

// `answer(args)`: the resources answer's lines.
function helper(answer) {
  fake.exec = (_cmd, args) => {
    if (args[0] === "version") return jl(header("version"), { type: "version", commands: ["version", "feed", "resources"] }, end());
    if (args[0] === "resources") return jl(header("resources"), ...answer(args));
    return jl(header(args[0]), { type: "end", ok: true, ms: 1 });
  };
}

const realNow = Date.now;
let clock = 0;
beforeEach(() => {
  reset(); forgetCommands(); R.forgetResources(); R.forgetCache(); calls.length = 0;
  fake.spaces = [space("sp1", { tabs: [{ id: "tb-t1", index: 1, name: "1", active: true, status: "idle", panes: 1, settled: false }] })];
  fake.agents = [agent("t1", "idle")];
  clock = realNow(); Date.now = () => clock;
});
afterEach(() => { Date.now = realNow; });

test("an open window stays open with no heartbeat: ticking for a minute writes nothing to shared", async () => {
  helper(() => [res("a", 10), end("c1", true)]);
  fake.shared[R.KEY_ON] = true; fake.shared[R.KEY_WIN_W] = 1400; fake.shared["flockbook.window.height"] = 900;
  command("resources");
  assert.equal(fake.shared[R.KEY_OPEN], "o1");
  fake.props = { view: "resources", overlay: "o1" };
  const cx = fakeCx();
  const v = new R.ResourcesView(cx);
  await cx.settle();
  v.tick(cx); await cx.settle();   // one refit to the item count it read
  log.shared.length = 0;
  for (let i = 0; i < 30; i++) { clock += 2000; v.tick(cx); await cx.settle(); }
  assert.equal(R.panelOpen(), true, "still open after a minute");
  assert.deepEqual(log.shared.map(([k]) => k), [], "no periodic shared write");
});

test("after a restart the id left in shared does not count as open, and the key opens a new window", async () => {
  fake.shared[R.KEY_ON] = true; fake.shared[R.KEY_WIN_W] = 1400; fake.shared["flockbook.window.height"] = 900;
  command("resources");
  assert.equal(R.panelOpen(), true);
  // The same shared (persisted), a new run of the module.
  const R2 = await import("../mods/flockbook/resources.js?restart");
  assert.equal(R2.panelOpen(), false, "the id is from the last run");
  assert.equal(R2.openFrame(), null);
  const n = log.overlays.length;
  R2.toggleResources();
  assert.equal(log.overlays.length, n + 1, "opened, not closed");
  assert.equal(R2.panelOpen(), true);
});

const remoteAgent = () => { fake.agents = [agent("box/t1", "idle", { tab: "tb-t1" })]; };
const err = (code, message) => ({ type: "error", code, message });

test("an agent on a machine whose helper is older: the window says so, not just 'No resources yet'", async () => {
  remoteAgent();
  helper(() => [err("remote_helper_outdated", "The helper on box is another version (0.0.9); this Flockbook needs 0.1.0."), { type: "end", ok: false, ms: 1 }]);
  await R.pollResources(fakeCx(), true);
  assert.equal(R.dataFor("box/t1")?.error, "old");
  fake.props = { view: "resources", overlay: "o7" };
  const el = new R.ResourcesView(fakeCx()).render(fakeCx());
  const empty = textOf(byId(el, "res-empty"));
  assert.match(empty, /helper on box is too old/i);
  assert.doesNotMatch(empty, /No resources yet/);
  // Updated there: the next answer clears it.
  helper(() => [end("c1", true)]);
  R.forgetResources();
  await R.pollResources(fakeCx(), true);
  assert.equal(R.dataFor("box/t1")?.error ?? null, null);
  assert.match(textOf(byId(new R.ResourcesView(fakeCx()).render(fakeCx()), "res-empty")), /No resources yet/);
});

test("no helper there: its own hint; another error keeps what was there and is not called too old", async () => {
  remoteAgent();
  helper(() => [err("remote_helper_missing", "box has no Flockbook helper yet."), { type: "end", ok: false, ms: 1 }]);
  await R.pollResources(fakeCx(), true);
  assert.equal(R.dataFor("box/t1")?.error, "missing");
  fake.props = { view: "resources", overlay: "o7" };
  assert.match(textOf(byId(new R.ResourcesView(fakeCx()).render(fakeCx()), "res-empty")), /box has no Flockbook helper/);
  // A passing failure (the machine did not answer): the hint stays as it was, nothing is written.
  log.shared.length = 0;
  helper(() => [err("remote_failed", "box did not answer"), { type: "end", ok: false, ms: 1 }]);
  R.forgetResources();
  await R.pollResources(fakeCx(), true);
  assert.equal(R.dataFor("box/t1")?.error, "missing");
  assert.deepEqual(log.shared.filter(([k]) => k === R.KEY_DATA), [], "the same hint again: not written again");
  // On this machine an error never gets the remote hint.
  assert.equal(R.applyAnswer(null, "t1", { error: { code: "read" }, lines: [] }), null);
});

// A chat whose tail starts at 9000; `older(before)` answers each --before read: [start, rows].
const feedRow = (id) => ({ type: "row", id, k: "assistant", text: id, ts: 1 });
function chatLog(older) {
  const asked = [];
  fake.exec = (_c, args) => {
    const i = args.indexOf("--before");
    if (i < 0) return jl(header("feed"), { type: "log", agent: "claude", path: "/logs/s.jsonl", start: 9000, size: 10, cursor: "2.x" },
      { type: "splice", at: 0, count: 1 }, feedRow("a9"), { type: "end", ok: true });
    const before = Number(args[i + 1]); asked.push(before);
    const [start, rows] = older(before);
    return jl(header("feed"), { type: "older", path: "/logs/s.jsonl", start, count: rows.length }, ...rows, { type: "end", ok: true });
  };
  return asked;
}
const ticks = async (v, cx, n = 10) => { for (let k = 0; k < n; k++) { await v.tick(cx); await cx.settle(); } };

test("Show in chat for a row older than what the chat loaded: the chat reads back to it, then shows it", async () => {
  const asked = chatLog((before) => (before === 9000 ? [6000, [feedRow("a6")]] : [3000, [feedRow("r-old"), feedRow("a3")]]));
  const cx = fakeCx(); const v = new ChatView(cx); await cx.settle();
  v.screen = "chat"; fake.shared["flockbook.screen"] = "chat";
  v.autoOlder = 5;   // the loads that fill a short chat are used up: only the jump reads back
  calls.length = 0;
  fake.shared[R.KEY_JUMP] = { terminal: "t1", row: "r-old", at: 5 };
  await ticks(v, cx);
  assert.deepEqual(asked, [9000, 6000]);
  const i = v.feed().rows.findIndex((r) => r.id === "r-old");
  assert.deepEqual(calls.filter((c) => c[0] === "scroll_to_item").at(-1), ["scroll_to_item", i, "top"]);
  assert.deepEqual(log.notify, [], "found: no hint");
});

test("Show in chat for a row the chat cannot find: a hint, not nothing", async () => {
  chatLog(() => [0, [feedRow("a0")]]);
  const cx = fakeCx(); const v = new ChatView(cx); await cx.settle();
  v.screen = "chat"; fake.shared["flockbook.screen"] = "chat"; v.autoOlder = 5;
  fake.shared[R.KEY_JUMP] = { terminal: "t1", row: "r-gone", at: 6 };
  await ticks(v, cx);
  assert.equal(log.notify.length, 1);
  assert.match(log.notify[0].title + " " + (log.notify[0].body ?? ""), /not .*in the chat|could not find/i);
  await ticks(v, cx, 3);
  assert.equal(log.notify.length, 1, "once");
});

test("Show in chat when the chat cannot read further back: the 'older than loaded' hint", async () => {
  chatLog(() => { throw new Error("read failed"); });
  fake.exec = ((inner) => (c, a) => (a.includes("--before") ? { code: 1, stdout: "", stderr: "boom" } : inner(c, a)))(fake.exec);
  const cx = fakeCx(); const v = new ChatView(cx); await cx.settle();
  v.screen = "chat"; fake.shared["flockbook.screen"] = "chat"; v.autoOlder = 5;
  fake.shared[R.KEY_JUMP] = { terminal: "t1", row: "r-old", at: 7 };
  await ticks(v, cx);
  assert.equal(log.notify.length, 1);
  assert.match(log.notify[0].body, /older than .*loaded/i);
});

test("Show in chat waits for the chat's first read before it looks", async () => {
  const cx = fakeCx();
  fake.exec = () => ({ code: 1, stdout: "", stderr: "slow" });
  const v = new ChatView(cx);
  v.feed().loaded = false;
  v.screen = "chat"; fake.shared["flockbook.screen"] = "chat";
  fake.shared[R.KEY_JUMP] = { terminal: "t1", row: "a9", at: 8 };
  v.lastPoll = Date.now();   // no poll this tick
  v.tick(cx);
  assert.deepEqual(log.notify, [], "not loaded yet: no hint");
  chatLog(() => [0, []]);
  v.lastPoll = 0;
  await ticks(v, cx, 3);
  assert.deepEqual(calls.filter((c) => c[0] === "scroll_to_item").at(-1), ["scroll_to_item", 0, "top"]);
  assert.deepEqual(log.notify, []);
});

// Codex R1 (631e057...90a3de0), the 3 Mediums.
test("an ask that comes back after BUSY_MS does not write over the newer answer", async () => {
  let releaseA = null, n = 0;
  fake.exec = (_c, args) => {
    if (args[0] === "version") return jl(header("version"), { type: "version", commands: ["version", "feed", "resources"] }, end());
    if (args[0] !== "resources") return jl(header(args[0]), { type: "end", ok: true, ms: 1 });
    n++;
    if (n === 1) return new Promise((done) => { releaseA = () => done(jl(header("resources"), res("a", 10), end("c1", true))); });
    return jl(header("resources"), res("b", 20), end("c2", true));
  };
  const cx = fakeCx();
  const a = R.pollResources(cx, true);
  for (let i = 0; i < 5 && !releaseA; i++) await new Promise((r) => setImmediate(r));
  assert.ok(releaseA, "A is waiting on the helper");
  clock += R.BUSY_MS + 1;
  await R.pollResources(cx, true);
  assert.equal(R.dataFor("t1")?.cursor, "c2");
  releaseA(); await a;   // A comes back late
  assert.equal(R.dataFor("t1")?.cursor, "c2", "A's older answer is dropped");
  assert.deepEqual(R.dataFor("t1").items.map((i) => i.id), ["b"]);
});

test("Show in chat from Overview: a chat made after the click still shows the row", async () => {
  const asked = chatLog((before) => (before === 9000 ? [6000, [feedRow("r-old"), feedRow("a6")]] : [0, []]));
  fake.shared["flockbook.screen"] = "chat";
  fake.shared[R.KEY_JUMP] = { terminal: "t1", row: "r-old", at: Date.now() };   // written just before the chat was made
  const cx = fakeCx(); const v = new ChatView(cx); v.autoOlder = 5; await cx.settle();
  calls.length = 0;
  await ticks(v, cx);
  assert.deepEqual(asked, [9000], "read back for the row");
  const i = v.feed().rows.findIndex((r) => r.id === "r-old");
  assert.deepEqual(calls.filter((c) => c[0] === "scroll_to_item").at(-1), ["scroll_to_item", i, "top"]);
});

test("a jump from long before the chat was made is not done again", async () => {
  chatLog(() => [0, []]);
  fake.shared["flockbook.screen"] = "chat";
  fake.shared[R.KEY_JUMP] = { terminal: "t1", row: "a9", at: Date.now() - 600000 };
  const cx = fakeCx(); const v = new ChatView(cx); await cx.settle();
  v.screen = "chat";
  calls.length = 0; v.follow = false;
  await ticks(v, cx, 3);
  assert.equal(v.pendingJump, null);
  assert.deepEqual(log.notify, []);
});

test("Show in chat when the chat gets no log: a hint after a while, not waiting for ever", async () => {
  fake.exec = (_c, args) => jl(header(args[0]), { type: "log", agent: "claude", path: null, start: null, size: 0, cursor: null }, { type: "end", ok: true });
  const cx = fakeCx(); const v = new ChatView(cx); await cx.settle();
  v.screen = "chat"; fake.shared["flockbook.screen"] = "chat";
  fake.shared[R.KEY_JUMP] = { terminal: "t1", row: "a9", at: Date.now() };
  await ticks(v, cx, 3);
  assert.deepEqual(log.notify, [], "not yet");
  clock += 60000;
  await ticks(v, cx, 2);
  assert.equal(log.notify.length, 1);
  assert.match(log.notify[0].title + " " + log.notify[0].body, /could not|not .*open|not found/i);
  assert.equal(v.pendingJump, null);
  await ticks(v, cx, 2);
  assert.equal(log.notify.length, 1, "once");
});

test("an older helper there (before resources) answers usage: the window says it is too old", async () => {
  remoteAgent();
  // What such a helper prints for "resources" (the same protocol and sidecar version, a shorter command list);
  // the helper here forwards it as it is (remote.rs forward).
  fake.exec = (_c, args) => {
    if (args[0] === "version") return jl(header("version"), { type: "version", commands: ["version", "feed", "resources"] }, end());
    if (args[0] === "resources") return jl(header("resources"), err("usage", "commands: version, feed, status, card, approve, answer, send"), { type: "end", ok: false, ms: 6 });
    return jl(header(args[0]), { type: "end", ok: true, ms: 1 });
  };
  await R.pollResources(fakeCx(), true);
  assert.equal(R.dataFor("box/t1")?.error, "old");
  fake.props = { view: "resources", overlay: "o7" };
  assert.match(textOf(byId(new R.ResourcesView(fakeCx()).render(fakeCx()), "res-empty")), /helper on box is too old/i);
  // On this machine the helper has the command (version says so): usage is not "too old".
  assert.equal(R.applyAnswer(null, "t1", { error: { code: "usage" }, lines: [] }), null);
});
