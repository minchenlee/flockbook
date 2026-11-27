// Resources load in parts (owner 11-27): the helper reads a long log newest first (`--chunk`); the window shows what
// is found so far, "Loading earlier… N%" under it, and "Reading this chat…" until the first part is in.
import { test, beforeEach } from "node:test";
import assert from "node:assert/strict";
import { fake, reset } from "./stub/herdr.mjs";
import { textOf, byId } from "./stub/ui.mjs";
import { fakeCx } from "./fake-cx.mjs";
import { agent, space } from "./fixtures.mjs";
import * as R from "../mods/flockbook/resources.js";
import { forgetCommands } from "../mods/flockbook/sidecar.js";
import { KEY_AGENT } from "../mods/flockbook/state.js";

const jl = (...lines) => ({ code: 0, stdout: lines.map((l) => JSON.stringify(l)).join("\n") + "\n", stderr: "" });
const header = (cmd) => ({ type: "header", protocol: 1, sidecar: "0.1.0", cmd });
const res = (id, last_ts) => ({ type: "res", id, kind: "link", target: `https://x.com/${id}`, title: id, detail: "x.com", row: `r-${id}`, first_ts: last_ts, last_ts, mentions: 1 });
const SIZE = 1000;
// The end line of a part (`read` bytes of SIZE so far) or of the last one.
const part = (cursor, read) => ({ type: "end", ok: true, ms: 1, cursor, full: true, more: true, read, size: SIZE });
const done = (cursor, full = true) => ({ type: "end", ok: true, ms: 1, cursor, full });

// A helper whose resources answers are `steps` in turn ([lines, end] or a whole exec result), then nothing new.
function helper(steps, seen = []) {
  const asked = [];
  let i = 0;
  fake.exec = (_cmd, args) => {
    if (args[0] === "version") return jl(header("version"), { type: "version", commands: ["version", "resources"] }, done());
    if (args[0] !== "resources") return jl(header(args[0]), { type: "end", ok: true, ms: 1 });
    asked.push(args);
    seen.push(R.dataFor(addr()));
    const s = steps[Math.min(i++, steps.length - 1)];
    return s.code !== undefined ? s : jl(header("resources"), ...s[0], s[1]);
  };
  return asked;
}
const addr = () => fake.agents[0].address;
const since = (args) => { const i = args.indexOf("--since"); return i < 0 ? null : args[i + 1]; };
const chunk = (args) => { const i = args.indexOf("--chunk"); return i < 0 ? null : args[i + 1]; };

beforeEach(() => {
  reset(); forgetCommands(); R.forgetResources(); R.forgetCache();
  fake.spaces = [space("sp1", { tabs: [{ id: "tb-t1", index: 1, name: "1", active: true, status: "idle", panes: 1, settled: false }] })];
  fake.agents = [agent("t1", "idle", { tab: "tb-t1" })];
});

const view = () => {
  fake.props = { view: "resources", overlay: "o7" };
  fake.shared[R.KEY_OPEN] = "o7"; fake.shared[R.KEY_RUN] = R.RUN; fake.shared[R.KEY_ON] = true;
  fake.shared[R.KEY_WIN_W] = 1400; fake.shared["flockbook.window.height"] = 900;
  return new R.ResourcesView(null);
};

test("one ask goes on part after part, newest first, each with --chunk and the last part's cursor", async () => {
  const seen = [];
  const asked = helper([
    [[res("n2", 90), res("n1", 80)], part("p1.a", 250)],
    [[res("n2", 90), res("n1", 80), res("o1", 40)], part("p1.b", 750)],
    [[res("n2", 90), res("n1", 80), res("o1", 40), res("o0", 10)], done("r4.z")],
  ], seen);
  await R.pollResources(fakeCx(), true);
  assert.equal(asked.length, 3, "all the parts in one ask");
  assert.deepEqual(asked.map(since), [null, "p1.a", "p1.b"]);
  assert.deepEqual(asked.map(chunk), [String(R.CHUNK), String(R.CHUNK), String(R.CHUNK)]);
  assert.equal(R.CHUNK, 8 * 1024 * 1024);
  // What the window had while the next part was read.
  assert.deepEqual(seen[1].items.map((i) => i.id), ["n2", "n1"]);
  assert.deepEqual(seen[1].loading, { read: 250, size: SIZE });
  assert.deepEqual(seen[2].loading, { read: 750, size: SIZE });
  const d = R.dataFor(addr());
  assert.deepEqual(d.items.map((i) => i.id), ["n2", "n1", "o1", "o0"]);
  assert.equal(d.loading, undefined, "done");
  assert.equal(d.cursor, "r4.z");
});

test("older parts never count as new: the badge stays 0 (first part with items, and first part empty)", async () => {
  helper([[[res("n1", 80)], part("p1.a", 300)], [[res("n1", 80), res("o1", 40)], done("r4.z")]]);
  await R.pollResources(fakeCx(), true);
  assert.equal(fake.shared[R.seenKey("t1")], 80);
  assert.equal(R.badgeFor(addr()), 0);

  reset(); forgetCommands(); R.forgetResources(); R.forgetCache();
  fake.spaces = [space("sp1", { tabs: [{ id: "tb-t1", index: 1, name: "1", active: true, status: "idle", panes: 1, settled: false }] })];
  fake.agents = [agent("t1", "idle", { tab: "tb-t1" })];
  const seenAt = [];
  helper([[[], part("p1.a", 300)], [[res("o1", 40)], part("p1.b", 600)], [[res("o1", 40), res("o0", 20)], done("r4.z")]], seenAt);
  await R.pollResources(fakeCx(), true);
  assert.equal(fake.shared[R.seenKey("t1")], 40, "set from the whole list (the first part had none)");
  assert.equal(R.badgeFor(addr()), 0);
  // A newer item after the load: that one is new.
  helper([[[res("x", 99)], done("r4.y", false)]]);
  await R.pollResources(fakeCx(), true);
  assert.equal(R.badgeFor(addr()), 1);
});

test("the window: Reading this chat… before the first part, then the list and Loading earlier… N%", async () => {
  const v = view();
  let el = v.render(fakeCx());
  assert.match(textOf(byId(el, "res-empty")), /Reading this chat…/, "no answer yet");
  assert.doesNotMatch(textOf(el), /No resources yet/);
  // A part with nothing in it: still reading.
  R.putData({ terminal: "t1", cursor: "p1.a", items: [], at: 1, loading: { read: 100, size: SIZE } });
  el = v.render(fakeCx());
  assert.match(textOf(byId(el, "res-empty")), /Reading this chat…/);
  R.putData({ terminal: "t1", cursor: "p1.b", items: [res("n1", 80)].map(({ type: _t, ...r }) => r), at: 1, loading: { read: 359, size: SIZE } });
  el = v.render(fakeCx());
  assert.ok(byId(el, "res-n1"), "the items found so far");
  assert.match(textOf(byId(el, "res-loading")), /Loading earlier… 35%/, "rounded down");
  assert.ok(byId(el, "res-list").kids.at(-1)._a.id === "res-loading", "at the end of the list");
  // Done.
  R.putData({ terminal: "t1", cursor: "r4.z", items: [res("n1", 80)].map(({ type: _t, ...r }) => r), at: 2 });
  el = v.render(fakeCx());
  assert.equal(byId(el, "res-loading"), null);
  R.putData({ terminal: "t1", cursor: "r4.z", items: [], at: 3 });
  assert.match(textOf(byId(v.render(fakeCx()), "res-empty")), /No resources yet/, "only when all of it is read");
});

test("no log: Can't find this chat's log; a log that cannot be read: Can't read this chat's log", async () => {
  helper([[[], { type: "end", ok: true, ms: 1, cursor: null, full: true }]]);
  await R.pollResources(fakeCx(), true);
  const v = view();
  assert.match(textOf(byId(v.render(fakeCx()), "res-empty")), /Can't find this chat's log/);
  R.forgetCache(); R.forgetResources();
  helper([jl(header("resources"), { type: "error", code: "read", message: "cannot read the log: denied" }, { type: "end", ok: false, ms: 1 })]);
  await R.pollResources(fakeCx(), true);
  assert.match(textOf(byId(v.render(fakeCx()), "res-empty")), /Can't read this chat's log/);
  // A read error after a list: the list stays.
  R.forgetCache(); R.forgetResources();
  helper([[[res("a", 5)], done("r4.a")], jl(header("resources"), { type: "error", code: "read", message: "x" }, { type: "end", ok: false, ms: 1 })]);
  await R.pollResources(fakeCx(), true);
  await R.pollResources(fakeCx(), true);
  assert.deepEqual(R.dataFor(addr()).items.map((i) => i.id), ["a"]);
});

test("a load left off (another agent shown) goes on from its cursor, also for a full read", async () => {
  fake.agents = [agent("t1", "idle", { tab: "tb-t1" }), agent("t2", "idle", { tab: "tb-t1" })];
  fake.shared[KEY_AGENT] = "t1";
  const asked = helper([
    [[res("n1", 80)], part("p1.a", 300)],
    [[res("n1", 80), res("o1", 40)], part("p1.b", 600)],
    [[res("n1", 80), res("o1", 40), res("o0", 10)], done("r4.z")],
  ]);
  const real = fake.exec;
  // After the first part, the shown agent is another one.
  fake.exec = (c, args) => { const r = real(c, args); if (args[0] === "resources") fake.shared[KEY_AGENT] = "t2"; return r; };
  await R.pollResources(fakeCx(), true);
  assert.equal(asked.length, 1, "stopped after the first part");
  assert.deepEqual(R.dataFor("t1").loading, { read: 300, size: SIZE });
  fake.shared[KEY_AGENT] = "t1"; fake.exec = real;
  await R.pollResources(fakeCx(), true, true);
  assert.equal(since(asked[1]), "p1.a", "goes on, not again from the end");
  assert.equal(asked.length, 3);
  assert.equal(R.dataFor("t1").loading, undefined);
  assert.deepEqual(R.dataFor("t1").items.map((i) => i.id), ["n1", "o1", "o0"]);
});

test("an older helper (no more, read whole): one ask, done", async () => {
  const asked = helper([[[res("a", 5), res("b", 9)], done("r4.a")]]);
  await R.pollResources(fakeCx(), true);
  assert.equal(asked.length, 1);
  assert.equal(R.dataFor(addr()).loading, undefined);
  assert.equal(fake.shared[R.seenKey("t1")], 9);
});

test("a part that reads nothing more stops the ask (the helper could not keep its state)", async () => {
  const asked = helper([
    [[res("n1", 80)], part("p1.a", 300)],
    [[res("n1", 80)], part("p1.a", 300)],
    [[res("n1", 80)], part("p1.a", 300)],
  ]);
  await R.pollResources(fakeCx(), true);
  assert.equal(asked.length, 2, "asked again once, then stopped");
});

test("a read error during a load keeps its place: the next ask goes on from the part's cursor", async () => {
  const asked = helper([
    [[], part("p1.a", 300)],
    jl(header("resources"), { type: "error", code: "read", message: "x" }, { type: "end", ok: false, ms: 1 }),
  ]);
  await R.pollResources(fakeCx(), true);
  assert.equal(R.dataFor(addr()).cursor, "p1.a");
  assert.deepEqual(R.dataFor(addr()).loading, { read: 300, size: SIZE });
  helper([[[res("o1", 40)], done("r4.z")]]).length;
  const again = helper([[[res("o1", 40)], done("r4.z")]]);
  await R.pollResources(fakeCx(), true, true);
  assert.equal(since(again[0]), "p1.a");
});

test("an empty first part: Reading this chat… and how far it is", () => {
  const v = view();
  R.putData({ terminal: "t1", cursor: "p1.a", items: [], at: 1, loading: { read: 120, size: SIZE } });
  const t = textOf(byId(v.render(fakeCx()), "res-empty"));
  assert.match(t, /Reading this chat…/);
  assert.match(t, /Loading earlier… 12%/);
});

test("while a part is read, a poll waits longer than BUSY_MS before it asks again", async () => {
  let release;
  const asked = [];
  fake.exec = (_c, args) => {
    if (args[0] === "version") return jl(header("version"), { type: "version", commands: ["version", "resources"] }, done());
    asked.push(args);
    if (asked.length === 1) return jl(header("resources"), res("n1", 80), part("p1.a", 300));
    return new Promise((r) => { release = () => r(jl(header("resources"), res("n1", 80), res("o1", 40), done("r4.z"))); });
  };
  const first = R.pollResources(fakeCx(), true);
  await new Promise((r) => setTimeout(r, 10));
  assert.equal(asked.length, 2, "the second part is being read");
  const real = Date.now;
  try {
    Date.now = () => real() + R.BUSY_MS + 1000;
    await R.pollResources(fakeCx(), true);
    assert.equal(asked.length, 2, "not asked again past BUSY_MS while a part is read");
  } finally { Date.now = real; }
  release();
  await first;
  assert.equal(R.dataFor(addr()).loading, undefined);
});
