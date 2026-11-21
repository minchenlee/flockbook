import { test, beforeEach } from "node:test";
import assert from "node:assert/strict";
import { fake, log, reset } from "./stub/herdr.mjs";
import { textOf, byId } from "./stub/ui.mjs";
import { fakeCx } from "./fake-cx.mjs";
import { agent, space } from "./fixtures.mjs";
import * as R from "../mods/flockbook/resources.js";
import { forgetCommands } from "../mods/flockbook/sidecar.js";
import { TopView, command } from "../mods/flockbook/top.js";
import { ChatView } from "../mods/flockbook/chat.js";
import { calls } from "./stub/ui.mjs";

const jl = (...lines) => ({ code: 0, stdout: lines.map((l) => JSON.stringify(l)).join("\n") + "\n", stderr: "" });
const header = (cmd) => ({ type: "header", protocol: 1, sidecar: "0.1.0", cmd });
const res = (id, kind, last_ts, extra = {}) => ({ type: "res", id, kind, target: extra.target ?? `https://x.com/${id}`, title: extra.title ?? id, detail: "x.com",
  row: `r-${id}`, first_ts: last_ts, last_ts, mentions: 1, ...extra });
const end = (cursor, full) => ({ type: "end", ok: true, ms: 1, cursor, full });
const answer = (lines, cursor = "c1", full = true) => ({ header: header("resources"), end: end(cursor, full), error: null, lines });

// A helper that has the command; `next` gives each resources answer's lines (and the args it got).
function helper(next, commands = ["version", "feed", "resources"]) {
  const asked = [];
  fake.exec = (_cmd, args) => {
    if (args[0] === "version") return jl(header("version"), { type: "version", commands }, end());
    if (args[0] === "resources") { asked.push(args); const [lines, cursor, full] = next(args); return jl(header("resources"), ...lines, end(cursor, full)); }
    return jl(header(args[0]), { type: "end", ok: true, ms: 1 });
  };
  return asked;
}

beforeEach(() => {
  reset(); forgetCommands(); R.forgetResources();
  fake.spaces = [space("sp1", { tabs: [{ id: "tb-t1", index: 1, name: "1", active: true, status: "idle", panes: 1, settled: false }] })];
  fake.agents = [agent("t1", "idle")];
});

test("an answer: full replaces, a delta merges by id, an error keeps what was there; newest first", () => {
  const a = R.applyAnswer(null, "t1", answer([res("a", "link", 10), res("b", "doc", 30)]));
  assert.deepEqual(a.items.map((i) => i.id), ["b", "a"]);
  assert.equal(a.cursor, "c1");
  assert.equal(a.items[0].type, undefined, "the line's type is not kept");
  const b = R.applyAnswer(a, "t1", answer([res("a", "link", 40, { mentions: 2 }), res("c", "pr", 35)], "c2", false));
  assert.deepEqual(b.items.map((i) => [i.id, i.last_ts]), [["a", 40], ["c", 35], ["b", 30]]);
  assert.equal(b.cursor, "c2");
  assert.equal(R.applyAnswer(b, "t1", { error: { code: "read" }, lines: [] }), b);
  const other = R.applyAnswer(b, "t2", answer([res("z", "link", 1)], "c9", false));
  assert.deepEqual(other.items.map((i) => i.id), ["z"], "another agent: not merged into this one's");
  assert.deepEqual(R.applyAnswer(b, "t1", answer([res("q", "link", 1)], "c3", true)).items.map((i) => i.id), ["q"]);
});

test("removed stays removed, also after a full read; mentioned again later it comes back", () => {
  const items = [res("a", "link", 10), res("b", "doc", 20)];
  const removed = [{ id: "a", last_ts: 10 }];
  assert.deepEqual(R.visible(items, removed).map((i) => i.id), ["b"]);
  // A full read again (a restart, a new cursor): the same last_ts, still removed.
  const again = R.applyAnswer(null, "t1", answer([res("a", "link", 10), res("b", "doc", 20)]));
  assert.deepEqual(R.visible(again.items, removed).map((i) => i.id), ["b"]);
  // Mentioned again: a newer last_ts.
  assert.deepEqual(R.visible([res("a", "link", 50), res("b", "doc", 20)], removed).map((i) => i.id), ["a", "b"]);
  assert.deepEqual(R.visible(items, ["a"]).map((i) => i.id), ["b"], "a bare id is removed for good");
  assert.deepEqual(R.visible([res("a", "link", 99)], ["a"]), []);
});

test("removeItem keeps one entry per id, with the last_ts it had", () => {
  R.removeItem(res("a", "link", 10), "t1");
  R.removeItem(res("a", "link", 20), "t1");
  assert.deepEqual(fake.shared[R.removedKey("t1")], [{ id: "a", last_ts: 20 }]);
});

test("the first look at an agent marks what is there as seen: no badge for an old conversation", async () => {
  let lines = [res("a", "link", 10), res("b", "doc", 20)];
  const asked = helper(() => [lines, "c1", true]);
  await R.pollResources(fakeCx(), true);
  assert.equal(fake.shared[R.KEY_ON], true);
  assert.equal(fake.shared[R.seenKey("t1")], 20);
  assert.equal(R.badgeFor("t1"), 0);
  assert.deepEqual(asked[0].slice(0, 3), ["resources", "--terminal", "t1"]);
  assert.ok(!asked[0].includes("--since"), "the first ask has no cursor");
  // The agent mentions a new one: the next ask passes the cursor; the badge counts it.
  lines = [res("c", "pr", 30)];
  helper(() => [lines, "c2", false]);
  await R.pollResources(fakeCx(), true);
  assert.ok(log.exec.at(-1)[1].includes("--since") && log.exec.at(-1)[1].includes("c1"));
  assert.equal(R.badgeFor("t1"), 1);
  R.removeItem(R.dataFor("t1").items[0], "t1");
  assert.equal(R.badgeFor("t1"), 0, "a removed one is not counted");
});

test("the window, when it opens, reads all again (a file written after it was mentioned)", async () => {
  const asked = helper(() => [[res("a", "doc", 10, { exists: false })], "c1", true]);
  await R.pollResources(fakeCx(), true);
  helper(() => [[res("a", "doc", 10, { exists: true })], "c1", true]);
  const cx = fakeCx();
  fake.props = { view: "resources", overlay: "o7" };
  new R.ResourcesView(cx);
  await cx.settle();
  const last = log.exec.at(-1)[1];
  assert.equal(last[0], "resources");
  assert.ok(!last.includes("--since"), String(last));
  assert.equal(R.dataFor("t1").items[0].exists, true);
  assert.equal(asked.length, 1);
});

test("asked at most every few seconds over all copies, and not at all by an older helper", async () => {
  const asked = helper(() => [[res("a", "link", 10)], "c1", true]);
  await R.pollResources(fakeCx(), true);
  await R.pollResources(fakeCx());
  assert.equal(asked.length, 1);
  reset(); forgetCommands(); R.forgetResources();
  fake.agents = [agent("t1", "idle")];
  const none = helper(() => [[], "c", true], ["version", "feed"]);
  await R.pollResources(fakeCx(), true);
  assert.equal(none.length, 0);
  assert.equal(fake.shared[R.KEY_ON], false);
});

test("what a click opens: web addresses in the browser; a doc file only when it is here and is a kind Flockbook opens", () => {
  assert.deepEqual(R.openPlan(res("a", "pr", 1, { target: "https://github.com/o/r/pull/1" }), "t1"), { url: "https://github.com/o/r/pull/1" });
  assert.ok(R.openPlan(res("a", "link", 1, { target: "javascript:alert(1)" }), "t1").why);
  const doc = (target, exists = true) => res("d", "doc", 1, { target, exists });
  assert.deepEqual(R.openPlan(doc("/home/me/plan.md"), "t1", "macos"), { cmd: "open", args: ["/home/me/plan.md"] });
  assert.deepEqual(R.openPlan(doc("/home/me/r.HTML"), "t1", "linux"), { cmd: "xdg-open", args: ["/home/me/r.HTML"] });
  assert.match(R.openPlan(doc("/home/me/plan.md", false), "t1").why, /not found/);
  assert.match(R.openPlan(doc("/home/me/plan.md"), "box/t1").why, /on box/, "an agent on another machine: its files are there");
  assert.match(R.openPlan(doc("/home/me/run.sh"), "t1").why, /does not open/);
  assert.match(R.openPlan(doc("~/plan.md"), "t1").why, /full path/);
});

// A helper whose open-check answers `check(path)` ({ ok, path } or { ok: false, why }).
function checker(check, commands = ["version", "resources", "open-check"]) {
  const asked = [];
  fake.exec = (_cmd, args) => {
    if (args[0] === "version") return jl(header("version"), { type: "version", commands }, end());
    if (args[0] === "open-check") { const p = args[args.indexOf("--path") + 1]; asked.push(p); return jl(header("open-check"), { type: "openable", ...check(p) }, { type: "end", ok: true, ms: 1 }); }
    return jl(header(args[0]), { type: "end", ok: true, ms: 1 });
  };
  return asked;
}

test("a click opens a file only after the helper says its real file opens, and opens the real file", async () => {
  const doc = (target, exists = true) => res("d", "doc", 1, { target, exists });
  const opened = [], ran = [];
  const cx = { open_url: (u) => opened.push(u) };
  const run = (c, a) => ran.push([c, a]);
  const asked = checker((p) => (p === "/home/me/a.md" ? { ok: true, path: "/real/b.md" } : { ok: false, why: "Flockbook does not open this kind of file." }));
  assert.ok((await R.openItem(res("a", "link", 1), "t1", cx, run)).url);
  assert.deepEqual(opened, ["https://x.com/a"]);
  // A link to a .txt that is named .md: the helper says no; nothing runs.
  const no = await R.openItem(doc("/home/me/preview.md"), "t1", cx, run);
  assert.match(no.why, /does not open/);
  // A link to a .md: the real file opens.
  const ok = await R.openItem(doc("/home/me/a.md"), "t1", cx, run);
  assert.deepEqual(ok, { cmd: "open", args: ["/real/b.md"] });
  // Not there (the cached exists): not even asked.
  assert.match((await R.openItem(doc("/home/me/a.md", false), "t1", cx, run)).why, /not found/);
  assert.deepEqual(asked, ["/home/me/preview.md", "/home/me/a.md"]);
  assert.deepEqual(ran, [["open", ["/real/b.md"]]]);
});

test("a helper without open-check opens no file", async () => {
  const asked = checker(() => ({ ok: true, path: "/real/b.md" }), ["version", "resources"]);
  const ran = [];
  const p = await R.openItem(res("d", "doc", 1, { target: "/home/me/a.md", exists: true }), "t1", {}, (c, a) => ran.push([c, a]));
  assert.match(p.why, /Update the Flockbook helper/);
  assert.deepEqual([asked, ran], [[], []]);
});

test("a helper that fails to answer opens no file", async () => {
  fake.exec = (_cmd, args) => (args[0] === "version" ? jl(header("version"), { type: "version", commands: ["version", "open-check"] }, end()) : { code: 1, stdout: "", stderr: "boom" });
  const ran = [];
  const p = await R.openItem(res("d", "doc", 1, { target: "/home/me/a.md", exists: true }), "t1", {}, (c, a) => ran.push([c, a]));
  assert.ok(p.why);
  assert.deepEqual(ran, []);
});

test("the window: under the bar's right edge, or a full-height drawer in a narrow window", () => {
  const wide = R.placement(1400, 900, 3, 2);
  assert.deepEqual(wide.at, { x: 1400 - 340 - 12, y: 60 });
  assert.equal(wide.width, 340);
  assert.ok(wide.height <= 900 - 72 && wide.height >= 180);
  assert.equal(R.placement(1400, 900, 500, 4).height, 900 - 72, "many: as tall as the window allows");
  const narrow = R.placement(700, 900, 3, 2);
  assert.deepEqual([narrow.at, narrow.height, narrow.drawer], [{ x: 360, y: 0 }, 900, true]);
});

test("⌘⇧R toggles the window; one left from before a restart does not count as open", () => {
  fake.shared[R.KEY_ON] = true;
  fake.shared[R.KEY_WIN_W] = 1000; fake.shared["flockbook.window.height"] = 800;
  command("resources");
  const spec = log.overlays.at(-1);
  assert.deepEqual([spec.mod, spec.kind, spec.frame, spec.key, spec.props.view, spec.width], ["flockbook", "window", "none", "flockbook.resources", "resources", 340]);
  assert.deepEqual(spec.at, { x: 1000 - 340 - 12, y: 60 });
  assert.equal(fake.shared[R.KEY_OPEN], "o1");
  command("resources");
  assert.equal(fake.shared[R.KEY_OPEN], null);
  assert.deepEqual(log.closed, ["o1"]);
  // A restart: the id is still in shared, but no window drew for a while.
  fake.shared[R.KEY_OPEN] = "old"; fake.shared[R.KEY_ALIVE] = Date.now() - 60_000;
  command("resources");
  assert.equal(fake.shared[R.KEY_OPEN], "o1", "opened, not closed");
  // The sidebar's width counts: the window is the bar plus the sidebar.
  fake.shared[R.KEY_OPEN] = null; fake.shared["flockbook.sidebar"] = true; fake.shared[R.KEY_SIDE_W] = 240;
  command("resources");
  assert.deepEqual(log.overlays.at(-1).at, { x: 1240 - 340 - 12, y: 60 });
  // An older helper: the key does nothing.
  fake.shared[R.KEY_ON] = false; fake.shared[R.KEY_OPEN] = null;
  const n = log.overlays.length;
  command("resources");
  assert.equal(log.overlays.length, n);
});

const view = (items, extra = {}) => {
  fake.shared[R.KEY_DATA] = { terminal: "t1", cursor: "c1", items: items.map(({ type: _t, ...r }) => r), at: 1 };
  fake.shared[R.seenKey("t1")] = extra.seen ?? 0;
  fake.props = { view: "resources", overlay: "o7" };
  fake.shared[R.KEY_OPEN] = "o7";
  return new R.ResourcesView(fakeCx());
};

test("the window: groups in order with counts, a new dot, a missing file struck, the footer, an empty state", () => {
  const now = Date.now();
  const v = view([res("l1", "link", now - 120_000), res("p1", "pr", now - 60_000, { title: "o/r#12" }), res("d1", "doc", now - 5_000, { title: "plan.md", detail: "~/notes", exists: true }),
    res("d2", "doc", now - 10_000, { title: "gone.md", detail: "~/x", exists: false }), res("a1", "artifact", now - 200_000), res("x", "link", now)], { seen: now - 7_000 });
  fake.shared[R.removedKey("t1")] = [{ id: "x", last_ts: now }];
  const el = v.render(fakeCx());
  const groups = ["docs", "artifacts", "github", "links"].map((g) => byId(el, `res-group-${g}`));
  assert.ok(groups.every(Boolean));
  assert.match(textOf(groups[0]), /^Docs 2 plan\.md now ~\/notes gone\.md/, "newest first in a group");
  assert.match(textOf(byId(el, "res-head")), /Resources.*This agent/);
  assert.ok(byId(el, "res-new-d1"), "newer than seen at open: a dot");
  assert.equal(byId(el, "res-new-d2"), null);
  assert.match(textOf(byId(el, "res-d2")), /File not found/);
  assert.match(textOf(byId(el, "res-p1")), /o\/r#12.*1m/);
  assert.equal(textOf(byId(el, "res-foot")), "5 items · 1 removed");
  assert.equal(byId(el, "res-x"), null, "removed: not listed");
  // Empty.
  const e = view([]).render(fakeCx());
  assert.match(textOf(e), /No resources yet.*Docs, artifacts and links the agent mentions in its replies show up here\./);
  assert.equal(textOf(byId(e, "res-foot")), "0 items");
});

test("shown is seen: the badge goes while the window is open", () => {
  const v = view([res("a", "link", 50)], { seen: 10 });
  assert.equal(R.badgeFor("t1"), 1);
  v.tick(fakeCx());
  assert.equal(R.badgeFor("t1"), 0);
  assert.ok(byId(v.render(fakeCx()), "res-new-a"), "the dot stays until the window opens again");
});

test("row actions: hover shows them; jump, copy and remove; a click opens; Esc and Close close", async () => {
  const v = view([res("a", "doc", 50, { target: "/home/me/a.md", exists: false })]);
  let el = v.render(fakeCx());
  assert.equal(byId(el, "res-acts-a"), null, "hidden until hovered");
  const cx = fakeCx(); cx.stop_propagation = () => {};
  byId(el, "res-a")._h.on_hover(true, cx);
  el = v.render(fakeCx());
  const copied = [];
  byId(el, "res-copy-a")._h.on_click({}, { ...cx, write_to_clipboard: (s) => copied.push(s) });
  assert.deepEqual(copied, ["/home/me/a.md"]);
  byId(el, "res-jump-a")._h.on_click({}, cx);
  assert.equal(fake.shared[R.KEY_JUMP].row, "r-a");
  assert.deepEqual(log.run.at(-1), ["layout.select", { stack: "screen", child: "chat" }]);
  // A click on a missing file says why, and runs nothing.
  const n = log.exec.length;
  byId(el, "res-a")._h.on_click({}, cx);
  await cx.settle();
  assert.equal(log.exec.length, n);
  assert.match(textOf(byId(v.render(fakeCx()), "res-a")), /File not found\./);
  byId(el, "res-remove-a")._h.on_click({}, cx);
  assert.deepEqual(fake.shared[R.removedKey("t1")], [{ id: "a", last_ts: 50 }]);
  // Esc.
  el = v.render(fakeCx());
  byId(el, "res-root")._h.on_key_down({ key: "escape" }, cx);
  assert.deepEqual(log.closed, ["o7"]);
  assert.equal(fake.shared[R.KEY_OPEN], null);
  v.close(cx);
  assert.equal(fake.shared[R.KEY_OPEN], null);
});

test("the chat scrolls to the row once, for its own agent", () => {
  const chat = new ChatView(fakeCx());
  const f = chat.feed();
  f.rows = [{ id: "r1", k: "user" }, { id: "r2", k: "assistant" }];
  calls.length = 0;
  fake.shared[R.KEY_JUMP] = { terminal: "t1", row: "r2", at: 5 };
  chat.screen = "chat"; fake.shared["flockbook.screen"] = "chat";
  chat.tick(fakeCx());
  assert.deepEqual(calls.filter((c) => c[0] === "scroll_to_item"), [["scroll_to_item", 1, "top"]]);
  calls.length = 0;
  chat.tick(fakeCx());
  assert.deepEqual(calls.filter((c) => c[0] === "scroll_to_item"), [], "once");
  fake.shared[R.KEY_JUMP] = { terminal: "t9", row: "r2", at: 6 };
  chat.tick(fakeCx());
  assert.deepEqual(calls.filter((c) => c[0] === "scroll_to_item"), [], "another agent's row");
});

test("the bar: a Resources button with a badge, only with a helper that has the command", () => {
  let el = new TopView(fakeCx()).render(fakeCx());
  assert.equal(byId(el, "resources"), null, "an older helper: no button");
  fake.shared[R.KEY_ON] = true;
  fake.shared[R.KEY_DATA] = { terminal: "t1", cursor: "c", items: [{ ...res("a", "link", 50) }, { ...res("b", "link", 40) }], at: 1 };
  fake.shared[R.seenKey("t1")] = 45;
  el = new TopView(fakeCx()).render(fakeCx());
  assert.ok(byId(el, "resources"));
  assert.equal(textOf(byId(el, "resources-badge")), "1");
  assert.ok(byId(el, "resources")._a.tooltip.includes("⌘⇧R"));
});
