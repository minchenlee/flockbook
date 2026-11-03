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
  assert.equal(o.layout.children[1].padding, 10, "the terminal sits inset from the frame");
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
