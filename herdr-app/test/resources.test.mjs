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
import { readFileSync } from "node:fs";
import { columnWidth } from "../mods/flockbook/estimate.js";

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

test("the window: under the bar's right edge, or a drawer down to the input in a narrow window", () => {
  const wide = R.placement(1600, 900, 3, 2);
  assert.deepEqual(wide.at, { x: 1600 - 320 - 12, y: 60 });
  assert.equal(wide.width, 320);
  assert.ok(wide.height <= 900 - 72 && wide.height >= 180);
  assert.equal(R.placement(1600, 900, 500, 4).height, 900 - R.INPUT_GUESS - 12 - 60, "many: down to the input (not measured yet: INPUT_GUESS) less 12");
  const narrow = R.placement(700, 900, 3, 2);
  assert.deepEqual([narrow.at, narrow.height, narrow.drawer], [{ x: 380, y: 0 }, 900 - R.INPUT_GUESS - 12, true]);
});

test("⌘⇧R toggles the window; one left from before a restart does not count as open", () => {
  fake.shared[R.KEY_ON] = true;
  fake.shared[R.KEY_WIN_W] = 1000; fake.shared["flockbook.window.height"] = 800;
  command("resources");
  const spec = log.overlays.at(-1);
  assert.deepEqual([spec.mod, spec.kind, spec.frame, spec.key, spec.props.view, spec.width], ["flockbook", "window", "none", undefined, "resources", 320]);
  assert.deepEqual(spec.at, { x: 1000 - 320 - 12, y: 60 });
  assert.equal(fake.shared[R.KEY_OPEN], "o1");
  command("resources");
  assert.equal(fake.shared[R.KEY_OPEN], null);
  assert.deepEqual(log.closed, ["o1"]);
  // A restart: the id is still in shared, but no window drew for a while.
  fake.shared[R.KEY_OPEN] = "old"; fake.shared[R.KEY_RUN] = "an earlier run";
  command("resources");
  assert.equal(fake.shared[R.KEY_OPEN], "o1", "opened, not closed");
  // The sidebar's width counts: the window is the bar plus the sidebar.
  fake.shared[R.KEY_OPEN] = null; fake.shared["flockbook.sidebar"] = true; fake.shared[R.KEY_SIDE_W] = 240;
  command("resources");
  assert.deepEqual(log.overlays.at(-1).at, { x: 1240 - 320 - 12, y: 60 });
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

test("the window: groups in order with counts, a new dot, a missing file struck, the count in the header, an empty state", () => {
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
  // The count is in the header (owner 11-25: no footer, more room for the list).
  assert.equal(byId(el, "res-foot"), null, "no footer");
  assert.equal(textOf(byId(byId(el, "res-head"), "res-count")), "5 items · 1 removed");
  assert.equal(byId(el, "res-x"), null, "removed: not listed");
  // Empty.
  const e = view([]).render(fakeCx());
  assert.match(textOf(e), /No resources yet.*Docs, artifacts and links the agent mentions in its replies show up here\./);
  assert.equal(byId(e, "res-foot"), null);
  assert.equal(textOf(byId(byId(e, "res-head"), "res-count")), "0 items");
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

test("a click on a web address row opens it with the click's cx", async () => {
  const v = view([res("a", "link", 50)]);
  const el = v.render(fakeCx());
  const opened = [];
  const cx = fakeCx(); cx.stop_propagation = () => {}; cx.open_url = (u) => opened.push(u);
  cx.spawn = (f) => { const p = f({ notify() {} }); cx.tasks.push(p); return p; };   // a task's cx has no open_url
  byId(el, "res-a")._h.on_click({}, cx);
  await cx.settle();
  assert.deepEqual(opened, ["https://x.com/a"]);
});

test("the chat scrolls to the row once, for its own agent", () => {
  const chat = new ChatView(fakeCx());
  const f = chat.feed();
  f.rows = [{ id: "r1", k: "user" }, { id: "r2", k: "assistant" }];
  f.loaded = true; f.path = "/logs/s.jsonl"; f.start = 0; f.first = true;   // read: the jump waits for the first read
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

// Where the reading column and the window are, in window px: a window `winW` wide with a sidebar `side` px wide
// shown left of the chat (0: hidden). The chat never moves for the window (owner 11-25).
function fit(winW, side, inputH = 0) {
  const paneW = winW - side;
  const p = R.placement(winW, 900, 5, 2, paneW, inputH);
  const r = R.readingLayout(paneW, { ...p, open: true });
  return { p, r, panelX: p.at.x, colRight: side + r.left + r.width };
}

test("the window open or closed: the reading column and the chat input do not move or narrow", () => {
  for (const [winW, side] of [[1470, 0], [1470, 250], [1100, 0], [1100, 250], [800, 0], [1800, 0], [700, 0]]) {
    const paneW = winW - side;
    const p = R.placement(winW, 900, 5, 2, paneW);
    assert.deepEqual(R.readingLayout(paneW, { ...p, open: true }), R.readingLayout(paneW, null), `${winW} wide, sidebar ${side}`);
  }
  const chat = new ChatView(fakeCx());
  chat.width = 850;
  const before = [chat.colWidth(), R.readingLayout(850).left];
  fake.shared[R.KEY_FRAME] = R.placement(1100, 900, 5, 2, 850);
  fake.shared[R.KEY_OPEN] = "o1"; fake.shared[R.KEY_RUN] = R.RUN;
  assert.deepEqual([chat.colWidth(), R.readingLayout(850).left], before);
});

test("the window's width: the room right of the column less 24 and 12, 260 to 320; less room: 320 over the chat", () => {
  // [window, sidebar] → [width, x, drawer]
  const want = [
    [[1470, 0], [319, 1139, false]],   // column ends at 1115: 355 px of room
    [[1600, 0], [320, 1268, false]],   // more room than 320: 320 at the right
    [[1470, 250], [320, 1138, false]], // column ends at 1240: 230 px, less than 260
    [[1100, 0], [320, 768, false]],
    [[800, 0], [320, 468, false]],
    [[700, 0], [320, 380, true]],      // under 720: a drawer
  ];
  for (const [[winW, side], [w, x, drawer]] of want) {
    const f = fit(winW, side);
    assert.deepEqual([f.p.width, f.p.at.x, f.p.drawer], [w, x, drawer], `${winW} wide, sidebar ${side}`);
    if (!drawer && f.p.width < R.PANEL_W) assert.ok(f.panelX - f.colRight >= R.READ_GAP, `${winW}: gap ${f.panelX - f.colRight}`);
  }
  // 1351 wide: the column ends at 1055, 260 px of room left; 1350: 259.
  assert.equal(fit(1351, 0).p.width, 260);
  assert.equal(fit(1350, 0).p.width, 320, "1 px short of 260: 320 over the chat");
});

test("the window placed now ends above the chat input's height (chat.js keeps it)", () => {
  fake.shared[R.KEY_INPUT_H] = 150; fake.shared[R.KEY_WIN_W] = 1470; fake.shared["flockbook.window.height"] = 900;
  fake.shared[R.KEY_DATA] = { terminal: "t1", cursor: "c", items: Array.from({ length: 30 }, (_, i) => res(`i${i}`, "link", 50 - i)), at: 1 };
  const p = R.placeNow();
  assert.ok(p.at.y + p.height <= 900 - 150 - 12, `bottom ${p.at.y + p.height}`);
});

test("the window's bottom stays 12 px above the chat input", () => {
  const f = fit(1470, 0, 150);
  assert.ok(f.p.at.y + f.p.height <= 900 - 150 - 12, `bottom ${f.p.at.y + f.p.height}`);
  const many = R.placement(1470, 900, 40, 6, 1470, 150);
  assert.equal(many.at.y + many.height, 900 - 150 - 12, "a long list: down to the input less 12");
  const small = R.placement(1470, 900, 1, 1, 1470, 150);
  assert.equal(small.height, R.placement(1470, 900, 1, 1).height, "a short list: as before");
});

test("refit: only a real change reopens; it does not shrink under the pointer", () => {
  const a = R.placement(1400, 900, 3, 1), more = R.placement(1400, 900, 8, 2), none = R.placement(1400, 900, 0, 0);
  assert.equal(R.needsRefit(a, a, false), false);
  assert.equal(R.needsRefit(null, a, false), false, "nothing open");
  assert.equal(R.needsRefit(a, more, false), true, "grows");
  assert.equal(R.needsRefit(a, more, true), true, "grows also under the pointer");
  assert.equal(R.needsRefit(a, none, false), true, "shrinks");
  assert.equal(R.needsRefit(a, none, true), false, "not while the pointer is on it");
  assert.equal(R.needsRefit(a, { ...a, height: a.height + 3 }, false), false, "a few px: no");
  assert.equal(R.needsRefit(a, R.placement(700, 900, 2, 1), false), true, "a drawer now");
  assert.equal(R.needsRefit(a, R.placement(1200, 900, 2, 1), false), true, "the window got narrower: another place");
});

test("the open window follows the item count and the shown agent: reopened in place, the keyboard not taken, dots kept", () => {
  fake.shared[R.KEY_WIN_W] = 1400; fake.shared["flockbook.window.height"] = 900;
  const v = view([res("a", "link", 50)], { seen: 10 });
  fake.shared[R.KEY_FRAME] = R.placement(1400, 900, 1, 1);
  fake.shared[R.KEY_RUN] = R.RUN;
  const n = log.overlays.length;
  v.tick(fakeCx());
  assert.equal(log.overlays.length, n, "nothing changed: not reopened");
  // Six more mentioned.
  fake.shared[R.KEY_DATA] = { ...fake.shared[R.KEY_DATA], items: ["b", "c", "d", "e", "f", "g", "a"].map((id, i) => res(id, "link", 60 - i)) };
  v.tick(fakeCx());
  assert.equal(log.overlays.length, n + 1);
  const spec = log.overlays.at(-1);
  assert.equal(spec.key, undefined, "no key: Herdr would keep a key's old place");
  assert.ok(spec.height > R.placement(1400, 900, 1, 1).height, "taller");
  assert.deepEqual([spec.props.view, spec.props.refit, spec.props.focus, spec.props.seenAtOpen], ["resources", true, false, 10]);
  assert.deepEqual(log.closed, ["o7"], "the old one closed, after the new one opened");
  assert.equal(fake.shared[R.KEY_FRAME].height, spec.height);
  v.close(fakeCx());   // the host tells the old one it closed
  assert.equal(fake.shared[R.KEY_OPEN], "o1", "the new window still counts as open");
  // Another agent in the tab, with nothing listed yet: the small height.
  fake.agents = [agent("t2", "idle", { tab: "tb-t1" })];
  v.tick(fakeCx());
  assert.equal(log.overlays.length, n + 2);
  assert.equal(log.overlays.at(-1).height, R.placement(1400, 900, 0, 0).height);
});

test("the window moves where the chat made room: a wider window, a second refit (Codex round 1 High)", () => {
  fake.shared[R.KEY_WIN_W] = 1100; fake.shared["flockbook.window.height"] = 900; fake.shared[R.KEY_ON] = true;
  fake.shared[R.KEY_DATA] = { terminal: "t1", cursor: "c1", items: [res("a", "link", 50)], at: 1 };
  command("resources");
  assert.deepEqual(log.shown.at(-1).at, { x: 1100 - 332, y: 60 });
  // Wider: the chat makes room at the new right edge, and the window must be there too.
  fake.shared[R.KEY_WIN_W] = 1470;
  fake.props = { view: "resources", overlay: "o1" };
  const v = new R.ResourcesView(fakeCx());
  v.tick(fakeCx());
  const want = R.placement(1470, 900, 1, 1);
  assert.deepEqual(log.shown.at(-1).at, want.at, "where the frame says");
  assert.deepEqual(fake.shared[R.KEY_FRAME].at, log.shown.at(-1).at, "the frame is where it is");
  // Narrower again.
  fake.shared[R.KEY_WIN_W] = 1100;
  v.tick(fakeCx());
  assert.deepEqual(log.shown.at(-1).at, { x: 1100 - 332, y: 60 });
});

test("the window cannot be dragged off its place: every press is stopped; close() without an id leaves the open one", () => {
  const v = view([res("a", "link", 50)]);
  const el = v.render(fakeCx());
  let stopped = 0;
  byId(el, "res-root")._h.on_mouse_down({}, { stop_propagation: () => stopped++ });
  assert.equal(stopped, 1);
  fake.props = { view: "resources" };
  fake.shared[R.KEY_OPEN] = "o9";
  v.close(fakeCx());
  assert.equal(fake.shared[R.KEY_OPEN], "o9", "not this window's to clear");
});

test("a refit window: no full read again, no focus unless it had it, the dots as they were", () => {
  fake.shared[R.KEY_DATA] = { terminal: "t1", cursor: "c1", items: [res("a", "link", 50)], at: 1 };
  fake.shared[R.seenKey("t1")] = 50;
  fake.props = { view: "resources", overlay: "o8", refit: true, focus: false, seenAtOpen: 10 };
  const cx = fakeCx();
  let focused = 0;
  cx.focus_handle = () => ({ focus: () => focused++, is_focused: () => false });
  const asked = helper(() => [[res("a", "link", 50)], "c2", false]);
  const v = new R.ResourcesView(cx);
  for (const t of cx.timers.filter((t) => t.once)) t.f(cx);
  assert.equal(focused, 0, "the keyboard stays where it is");
  assert.equal(v.seenAtOpen, 10);
  assert.ok(byId(v.render(fakeCx()), "res-new-a"), "the dot stays");
  return cx.settle().then(() => assert.ok(asked.every((a) => a.includes("--since")), "no full read"));
});

test("row actions use icons that say what they do", () => {
  const v = view([res("a", "link", 50)]);
  const cx = fakeCx(); cx.stop_propagation = () => {};
  byId(v.render(fakeCx()), "res-a")._h.on_hover(true, cx);
  const el = v.render(fakeCx());
  const iconOf = (id) => byId(el, id).kids[0]._a.path;
  assert.match(iconOf("res-jump-a"), /\/icons\/message\.svg$/);
  assert.match(iconOf("res-copy-a"), /\/icons\/copy\.svg$/);
  assert.match(iconOf("res-remove-a"), /\/icons\/eye-off\.svg$/);
  assert.match(iconOf("res-close"), /\/icons\/close\.svg$/, "close the window: still the ×");
  assert.deepEqual(["res-jump-a", "res-copy-a", "res-remove-a"].map((id) => byId(el, id)._a.tooltip), ["Show in chat", "Copy", "Remove from list"]);
  for (const name of ["message", "copy", "eye-off"]) {
    const s = readFileSync(new URL(`../mods/flockbook/icons/${name}.svg`, import.meta.url), "utf8");
    assert.match(s, /Lucide.*ISC/, `${name}: says where it is from`);
    assert.match(s, /viewBox="0 0 24 24"/);
    assert.match(s, /stroke-width="2\.2"/, `${name}: as thin as the other icons (1.5 of 16)`);
  }
});

test("a command: its group, a click copies it (never runs it), hover has Show in chat and Remove, not Copy", async () => {
  const c = res("c1", "cmd", 50, { target: "cd ~/x && \\\n  make", title: "Try (one line)", detail: "cd ~/x && make" });
  assert.deepEqual(R.openPlan(c, "t1"), { copy: "cd ~/x && \\\n  make" });
  assert.deepEqual(R.grouped([c, res("l", "link", 1)]).map((g) => g.label), ["Commands", "Links"]);
  const v = view([c]);
  let el = v.render(fakeCx());
  assert.match(textOf(byId(el, "res-group-commands")), /^Commands 1 Try \(one line\)/);
  const copied = [], opened = [];
  const cx = fakeCx(); cx.stop_propagation = () => {}; cx.write_to_clipboard = (s) => copied.push(s); cx.open_url = (u) => opened.push(u);
  const n = log.exec.length;
  byId(el, "res-c1")._h.on_click({}, cx);
  await cx.settle();
  assert.deepEqual(copied, ["cd ~/x && \\\n  make"]);
  assert.deepEqual([opened, log.exec.length], [[], n], "nothing opens or runs");
  assert.match(textOf(byId(v.render(fakeCx()), "res-c1")), /Copied/);
  byId(el, "res-c1")._h.on_hover(true, cx);
  el = v.render(fakeCx());
  assert.ok(byId(el, "res-jump-c1") && byId(el, "res-remove-c1"));
  assert.equal(byId(el, "res-copy-c1"), null, "the click copies");
});


// The chat as the person sees it, with one row: the tree ChatView renders.
async function chatTree() {
  fake.exec = (_cmd, args) => args[0] === "feed"
    ? jl(header("feed"), { type: "log", agent: "claude", path: "/l.jsonl", start: 0, size: 1, cursor: "c" }, { type: "splice", at: 0, count: 1 }, { type: "row", id: "u1", k: "user", text: "hi" }, end())
    : jl(header(args[0]), end());
  const cx = fakeCx(); const v = new ChatView(cx); await cx.settle();
  return { v, cx, root: v.render(cx) };
}
// The size props (padding, height, gap, border) of every element under `el`, by path.
const SIZE = ["p", "px", "py", "pt", "pb", "pl", "pr", "m", "mx", "my", "mt", "mb", "h", "min_h", "max_h", "gap", "border", "w", "max_w", "size"];
function geo(el, path = []) {
  if (!el || typeof el !== "object") return [];
  const here = [...path, el._a?.id ?? el.tag];
  const own = SIZE.filter((k) => k in (el._a ?? {})).map((k) => `${k}=${JSON.stringify(el._a[k])}`);
  return [...(own.length ? [`${here.join("/")} ${own.join(" ")}`] : []), ...(el.kids ?? []).flatMap((k) => geo(k, here))];
}

test("the drawer and the window ends 12 px above the chat input; not measured yet: INPUT_GUESS (Codex R1 High)", () => {
  // 700 x 900, a drawer, the input 150 high: its bottom at 738 at most, not 900.
  const d = R.placement(700, 900, 40, 6, 700, 150);
  assert.equal(d.drawer, true);
  assert.equal(d.at.y + d.height, 900 - 150 - 12);
  // 800 x 900, a long list, the input not measured yet: not down to 888.
  const w = R.placement(800, 900, 40, 6, 800, 0);
  assert.equal(w.at.y + w.height, 900 - R.INPUT_GUESS - 12);
  const dn = R.placement(700, 900, 40, 6, 700, 0);
  assert.equal(dn.at.y + dn.height, 900 - R.INPUT_GUESS - 12);
  assert.ok(R.INPUT_GUESS >= 150, "the guess is not below the one-line input's height");
});

test("the input's height measured on the rendered chat moves the window's bottom, in a drawer too (Codex R1 High)", async () => {
  const { root, cx } = await chatTree();
  const wrap = byId(root, "composer-size");
  assert.ok(wrap?._h.on_size_change, "the measured element has the stable id Herdr needs for on_size_change");
  assert.ok(byId(wrap, "composer"), "it holds the chat input");
  fake.shared[R.KEY_DATA] = { terminal: "t1", cursor: "c", items: Array.from({ length: 40 }, (_, i) => res(`i${i}`, "link", 50 - i)), at: 1 };
  fake.shared["flockbook.window.height"] = 900;
  for (const [winW, inputH] of [[700, 150], [700, 270], [800, 150], [1470, 270]]) {
    fake.shared[R.KEY_WIN_W] = winW;
    wrap._h.on_size_change({ width: winW, height: inputH }, cx);
    const p = R.placeNow();
    assert.equal(p.at.y + p.height, 900 - inputH - 12, `${winW} wide, input ${inputH}: bottom ${p.at.y + p.height}`);
  }
});

test("the window open or closed: the rendered chat is the same, and the input keeps its size (Codex R1 High)", async () => {
  const closed = geo((await chatTree()).root);
  fake.shared[R.KEY_WIN_W] = 1100; fake.shared["flockbook.window.height"] = 900; fake.shared[R.KEY_INPUT_H] = 150;
  fake.shared[R.KEY_FRAME] = R.placement(1100, 900, 5, 2, 1100, 150);
  fake.shared[R.KEY_OPEN] = "o1"; fake.shared[R.KEY_RUN] = R.RUN;
  assert.equal(R.openFrame() !== null, true, "the window counts as open");
  assert.deepEqual(geo((await chatTree()).root), closed, "nothing in the chat moves or grows for the window");
  // The input's own sizes: INPUT_GUESS is about this input. A change here (more padding, a taller row) must
  // measure the input again and check INPUT_GUESS.
  assert.deepEqual(geo(byId((await chatTree()).root, "composer-size")), [
    "composer-size/composer px=16 pt=6 pb=14 gap=6",
    "composer-size/composer/tags gap=5 max_w=800",
    "composer-size/composer/tags/h px=8 h=22 gap=4 border=1",
    "composer-size/composer/tags/h/div max_w=220",
    "composer-size/composer/box px=12 pt=10 pb=8 gap=6 border=1 max_w=800",
    "composer-size/composer/box/h gap=8",
    "composer-size/composer/box/h/attach size=28",
    "composer-size/composer/box/h/attach/svg size=15",
    "composer-size/composer/box/h/send size=28",
    "composer-size/composer/box/h/send/svg size=14",
  ]);
});

test("an ask that never comes back (its window closed while it waited) does not stop the next asks", async () => {
  // The first ask hangs forever: the task that ran it is dropped, so its finally never runs.
  fake.exec = (_cmd, args) => {
    if (args[0] === "version") return jl(header("version"), { type: "version", commands: ["version", "feed", "resources"] }, end());
    return new Promise(() => {});
  };
  R.pollResources(fakeCx(), true);
  await new Promise((r) => setImmediate(r));
  // Later (past the wait limit): it is asked again, and the items show.
  const realNow = Date.now;
  Date.now = () => realNow() + R.BUSY_MS + 1;
  try {
    const asked = helper(() => [[res("a", "doc", 10)], "c1", true]);
    await R.pollResources(fakeCx());
    assert.equal(asked.length, 1, "the stuck ask does not block for good");
    assert.equal(R.dataFor("t1")?.items.length, 1);
  } finally { Date.now = realNow; }
});
