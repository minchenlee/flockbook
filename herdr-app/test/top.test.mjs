import { test, beforeEach } from "node:test";
import assert from "node:assert/strict";
import { fake, log, reset } from "./stub/herdr.mjs";
import { textOf, byId } from "./stub/ui.mjs";
import { fakeCx } from "./fake-cx.mjs";
import { agent, space } from "./fixtures.mjs";
import { TopView } from "../mods/flockbook/top.js";
import { KEY_AGENT } from "../mods/flockbook/state.js";

const tab = (id, index, extra = {}) => ({ id, index, name: String(index), named: false, active: false, status: null, panes: 1, zoomed: false, settled: false, settled_at: null, changed_at: null, working_since: null, ...extra });

beforeEach(() => {
  reset();
  fake.spaces = [space("sp1", { tabs: [tab("tb-t1", 1, { active: true, status: "working", name: "api", named: true }), tab("tb-t2", 2, { status: "done" }),
    tab("tb-t3", 3, { status: "blocked" }), tab("tb-sh", 4), tab("tb-gone", 5, { settled: true })] })];
  fake.agents = [agent("t1", "working", { tab_name: "api" }), agent("t2", "done"), agent("t3", "blocked", { codex: true, program: { key: "codex", name: "Codex", icon: "icons/codex.svg", tint: true, agent: true } }),
    agent("t9", "idle", { space: "sp2" })];
  fake.layout = { id: "root", traffic_lights: [18, 22], children: [{ id: "sidebar", hidden: true }, { id: "screen", active: "chat" }] };
});

test("one tab per tab of the shown space: named, else its agent, else its number", () => {
  const el = new TopView(fakeCx()).render(fakeCx());
  assert.ok(byId(el, "tab-tb-t1") && byId(el, "tab-tb-t2") && byId(el, "tab-tb-t3") && byId(el, "tab-tb-sh"));
  assert.equal(byId(el, "tab-tb-gone"), null, "a settled tab has no pill");
  assert.match(textOf(byId(el, "tab-tb-t1")), /api/);
  assert.match(textOf(byId(el, "tab-tb-t3")), /Codex/);
  assert.match(textOf(byId(el, "tab-tb-sh")), /^4/);
  assert.deepEqual(log.order, ["t1", "t2", "t3"]);
});

test("status marks: spinner while working, check when done, dot when blocked, ring when idle", () => {
  const el = new TopView(fakeCx()).render(fakeCx());
  const kids = (id) => byId(el, id).kids;
  assert.ok(kids("tab-tb-t1").some((k) => k.tag === "svg" && k._a.path.endsWith("spinner.svg") && k._a.spin === 600));
  assert.ok(kids("tab-tb-t2").some((k) => k.tag === "svg" && k._a.path.endsWith("check.svg")));
  assert.ok(kids("tab-tb-t3").some((k) => k.tag === "div" && k._a.bg === "#blocked"));
  assert.ok(kids("tab-tb-sh").some((k) => k.tag === "div" && k._a.border_color === "#dim"));
});

test("a click on a tab selects it: the chat for an agent tab, its terminals for a shell tab", () => {
  const el = new TopView(fakeCx()).render(fakeCx());
  byId(el, "tab-tb-t2")._h.on_click({}, fakeCx());
  assert.deepEqual(log.run, [["tab.select", { id: "tb-t2" }], ["layout.select", { stack: "screen", child: "chat" }]]);
  log.run = [];
  byId(el, "tab-tb-sh")._h.on_click({}, fakeCx());
  assert.deepEqual(log.run, [["tab.select", { id: "tb-sh" }], ["layout.select", { stack: "screen", child: "tab" }]]);
});

test("the screen follows the shown tab when it changes (keys, new tab), not otherwise", () => {
  const top = new TopView(fakeCx());
  top.sync();
  assert.deepEqual(log.run, [["layout.select", { stack: "screen", child: "chat" }]], "first look: the agent tab's chat");
  log.run = [];
  top.sync();
  assert.deepEqual(log.run, [], "a screen picked by hand stays while the tab stays");
  fake.spaces[0].tabs.forEach((x) => { x.active = x.id === "tb-sh"; });
  top.sync();
  assert.deepEqual(log.run, [["layout.select", { stack: "screen", child: "tab" }]]);
  fake.agents.push(agent("t5", "idle", { tab: "tb-sh" }));
  top.sync();
  assert.deepEqual(log.run.at(-1), ["layout.select", { stack: "screen", child: "chat" }], "an agent started in the shell tab");
});

test("key commands: move the tab, open the terminal, Needs you", () => {
  const top = new TopView(fakeCx());
  top.command("move-right");
  assert.deepEqual(log.run.at(-1), ["tab.move", { id: "tb-t1", index: 2 }]);
  log.run = [];
  top.command("move-left");
  assert.deepEqual(log.run, [], "the first tab cannot move left");
  top.command("terminal");
  assert.deepEqual(log.overlays.at(-1).layout.children[1].content, { terminal: "t1" });
  top.command("needs");
  assert.deepEqual(log.run.at(-1), ["layout.select", { stack: "screen", child: "needs" }]);
});

test("the sidebar button shows and hides the sidebar; with it open the button lives in the sidebar", async () => {
  let el = new TopView(fakeCx()).render(fakeCx());
  assert.equal(byId(el, "needs"), null, "no Needs you in the top bar");
  byId(el, "sidebar")._h.on_click({}, fakeCx());
  assert.deepEqual(log.run.at(-1), ["layout.show", { id: "sidebar" }]);
  el = new TopView(fakeCx()).render(fakeCx());
  assert.equal(byId(el, "sidebar"), null, "open: no button in the bar");
  const { SideView } = await import("../mods/flockbook/side.js");
  byId(new SideView(fakeCx()).render(fakeCx()), "side-toggle")._h.on_click({}, fakeCx());
  assert.deepEqual(log.run.at(-1), ["layout.hide", { id: "sidebar" }]);
  byId(el, "new")._h.on_click({}, fakeCx());
  assert.equal(log.overlays.at(-1).mod, "new-menu");
});

test("the sidebar state starts from the app's saved layout state", async () => {
  const { loadSidebar, sidebarShown } = await import("../mods/flockbook/state.js");
  const h = await import("./stub/herdr.mjs");
  fake.readText = (p) => (p.endsWith("/layouts/.state/flockbook.json") ? JSON.stringify({ nodes: { sidebar: { hidden: false } } }) : null);
  await loadSidebar();
  assert.equal(sidebarShown(), true);
  fake.readText = null;
});

test("no tabs: a plain note, and + stays", () => {
  fake.agents = []; fake.spaces[0].tabs = [];
  const el = new TopView(fakeCx()).render(fakeCx());
  assert.match(textOf(el), /No tabs in this space/);
  assert.ok(byId(el, "new"));
});

test("only the shown tab has a close button; it closes that tab without selecting it", () => {
  const el = new TopView(fakeCx()).render(fakeCx());
  assert.equal(byId(el, "close-tb-t2"), null);
  let stopped = false;
  byId(el, "close-tb-t1")._h.on_click({}, { ...fakeCx(), stop_propagation() { stopped = true; } });
  assert.deepEqual(log.run, [["tab.close", { id: "tb-t1" }]]);
  assert.ok(stopped);
});

test("the chat shows an agent of the shown tab, none for a shell tab", async () => {
  const { currentAgent } = await import("../mods/flockbook/state.js");
  fake.shared[KEY_AGENT] = "t2";
  assert.equal(currentAgent().address, "t1", "a pick in another tab is ignored");
  fake.spaces[0].tabs.forEach((x) => { x.active = x.id === "tb-sh"; });
  assert.equal(currentAgent(), null);
});

test("Terminal opens the agent's own terminal in a window with a Close bar", async () => {
  const el = new TopView(fakeCx()).render(fakeCx());
  byId(el, "terminal")._h.on_click({}, fakeCx());
  const o = log.overlays.at(-1);
  assert.equal(o.kind, "window"); assert.equal(o.key, "flockbook-terminal");
  assert.deepEqual(o.layout.children[1].content, { terminal: "t1" });
  assert.deepEqual(o.layout.children[0].content.props, { view: "termbar", terminal: "t1", name: "api" });
  const { TermBar } = await import("../mods/flockbook/termbar.js");
  fake.props = { view: "termbar", name: "api", overlay: "o9" };
  const bar = new TermBar(fakeCx()).render(fakeCx());
  assert.match(textOf(bar), /api.*Terminal/);
  assert.equal(o.layout.children[1].padding, undefined, "no inset around the terminal");
  assert.equal(o.frame, "none"); assert.equal(o.layout.radius, 16);
  assert.ok(byId(bar, "term-close"));
});

test("ctrl-` / Terminal toggles: a second press closes the window; Close forgets it", async () => {
  const el = new TopView(fakeCx()).render(fakeCx());
  byId(el, "terminal")._h.on_click({}, fakeCx());
  assert.equal(log.overlays.length, 1); assert.equal(fake.shared["flockbook.terminal"], "o1");
  byId(el, "terminal")._h.on_click({}, fakeCx());
  assert.equal(log.overlays.length, 1, "not opened again"); assert.deepEqual(log.closed, ["o1"]);
  assert.equal(fake.shared["flockbook.terminal"], null);
  byId(el, "terminal")._h.on_click({}, fakeCx());
  const { TermBar } = await import("../mods/flockbook/termbar.js");
  fake.props = { view: "termbar", name: "api", overlay: "o1" };
  byId(new TermBar(fakeCx()).render(fakeCx()), "term-close")._h.on_click({}, fakeCx());
  assert.equal(fake.shared["flockbook.terminal"], null);
});

test("a shell tab: no Terminal button", () => {
  fake.spaces[0].tabs.forEach((x) => { x.active = x.id === "tb-sh"; });
  assert.equal(byId(new TopView(fakeCx()).render(fakeCx()), "terminal"), null);
});

// Tab drag. Each pill is 80 px wide, 10 px apart, from x = 100: tb-t1, tb-t2, tb-t3, tb-sh.
const X = { "tb-t1": 100, "tb-t2": 190, "tb-t3": 280, "tb-sh": 370 };
const at = (id, dx) => ({ button: "left", click_count: 1, modifiers: {}, position: { x: X[id] + dx, y: 20 },
  local_position: { x: dx, y: 10 }, bounds: { x: X[id], y: 10, width: 80, height: 30 } });
const pill = (el, id) => byId(el, `tab-${id}`);
function press(el, id, dx = 20) { pill(el, id)._h.on_mouse_down(at(id, dx), fakeCx()); }
function move(el, id, dx) { pill(el, id)._h.on_mouse_move(at(id, dx), fakeCx()); }
function release(el, id, dx) { byId(el, "tab-scroll")._h.on_mouse_up(at(id, dx), fakeCx()); }
const render = () => new TopView(fakeCx()).render(fakeCx());

test("drag a tab right: the line shows after the tab under the pointer, release moves it there once", () => {
  let el = render();
  press(el, "tb-t1"); move(el, "tb-t1", 30); move(el, "tb-t3", 60);
  el = render();
  const line = byId(pill(el, "tb-t3"), "drop-line");
  assert.ok(line, "the insertion line is on tb-t3");
  assert.ok(line._a.right !== undefined && line._a.left === undefined, "on its right side");
  assert.equal(byId(el, "drop-line") === line, true, "one line only");
  assert.ok(pill(el, "tb-t1")._a.opacity < 1, "the dragged tab is dimmed");
  release(el, "tb-t3", 60);
  assert.deepEqual(log.run, [["tab.move", { id: "tb-t1", index: 3 }]]);
  assert.equal(byId(render(), "drop-line"), null, "the line goes after the drop");
});

test("drag a tab left: before the tab under the pointer", () => {
  let el = render();
  press(el, "tb-sh"); move(el, "tb-sh", 10); move(el, "tb-t2", 15);
  el = render();
  assert.ok(byId(pill(el, "tb-t2"), "drop-line")._a.left !== undefined, "on tb-t2's left side");
  release(el, "tb-t2", 15);
  assert.deepEqual(log.run, [["tab.move", { id: "tb-sh", index: 2 }]]);
});

test("the place is counted from the pointer on the pill, so a scrolled bar gives the same place", () => {
  let el = render();
  const shift = (e) => ({ ...e, position: { x: e.position.x - 250, y: 20 }, bounds: { ...e.bounds, x: e.bounds.x - 250 } });
  pill(el, "tb-t1")._h.on_mouse_down(shift(at("tb-t1", 20)), fakeCx());
  pill(el, "tb-t1")._h.on_mouse_move(shift(at("tb-t1", 30)), fakeCx());
  pill(el, "tb-t2")._h.on_mouse_move(shift(at("tb-t2", 70)), fakeCx());
  byId(el, "tab-scroll")._h.on_mouse_up(shift(at("tb-t2", 70)), fakeCx());
  assert.deepEqual(log.run, [["tab.move", { id: "tb-t1", index: 2 }]]);
});

test("a press that moves less than 4 px is a click: the tab is shown, nothing moves", () => {
  const el = render();
  press(el, "tb-t2"); move(el, "tb-t2", 23);
  assert.equal(byId(render(), "drop-line"), null);
  pill(el, "tb-t2")._h.on_click({}, fakeCx());
  release(el, "tb-t2", 23);
  assert.deepEqual(log.run.map((r) => r[0]), ["tab.select", "layout.select"]);
});

test("a drag back to its own place moves nothing and does not select the tab", () => {
  const el = render();
  press(el, "tb-t2"); move(el, "tb-t2", 70); move(el, "tb-t1", 70);
  assert.equal(byId(render(), "drop-line"), null, "right after tb-t1 is where tb-t2 already is");
  pill(el, "tb-t2")._h.on_click({}, fakeCx());
  release(el, "tb-t1", 70);
  assert.deepEqual(log.run, []);
});

test("leaving the tab bar cancels: no line, the tab is not dimmed, the release moves nothing", () => {
  let el = render();
  press(el, "tb-t1"); move(el, "tb-t1", 30); move(el, "tb-t3", 60);
  byId(el, "tabs")._h.on_hover(false, fakeCx());
  el = render();
  assert.equal(byId(el, "drop-line"), null);
  assert.equal(pill(el, "tb-t1")._a.opacity, undefined);
  release(el, "tb-t3", 60);
  move(el, "tb-t3", 60);
  assert.equal(byId(render(), "drop-line"), null, "moving over it again with no press starts nothing");
  assert.deepEqual(log.run, []);
});

test("another mouse button, or a press outside the bar, cancels the drag", () => {
  let el = render();
  press(el, "tb-t1"); move(el, "tb-t1", 30); move(el, "tb-t3", 60);
  pill(el, "tb-t3")._h.on_mouse_down_right(at("tb-t3", 60), { ...fakeCx(), stop_propagation() {} });
  assert.equal(log.overlays.length, 0, "the right press only cancels; no menu during a drag");
  release(el, "tb-t3", 60);
  assert.deepEqual(log.run, []);
  el = render();
  press(el, "tb-t1"); move(el, "tb-t1", 30); move(el, "tb-t3", 60);
  byId(el, "tabs")._h.on_mouse_down_out(at("tb-t3", 60), fakeCx());
  release(el, "tb-t3", 60);
  assert.deepEqual(log.run, []);
});
