import { test, beforeEach } from "node:test";
import assert from "node:assert/strict";
import { fake, log, reset } from "./stub/herdr.mjs";
import { textOf, byId } from "./stub/ui.mjs";
import { fakeCx } from "./fake-cx.mjs";
import { agent, space } from "./fixtures.mjs";
import { tabItems, spaceItems, openMenu, act, MenuView } from "../mods/flockbook/menu.js";
import { TopView } from "../mods/flockbook/top.js";
import { screenFor } from "../mods/flockbook/state.js";

const tab = (id, index, extra = {}) => ({ id, index, name: String(index), named: false, active: false, status: null, panes: 1, settled: false, ...extra });

beforeEach(() => {
  reset();
  fake.agents = [agent("t1", "idle", { tab: "tb1" })];
  fake.spaces = [space("sp1", { tabs: [tab("tb1", 1, { active: true }), tab("tb2", 2)] })];
});

test("a tab's menu: rename, move, terminal, panes, splits, put away, close; ends of the row lose a move", () => {
  const list = fake.spaces[0].tabs;
  const labels = (items) => items.filter(Boolean).map((i) => i.label);
  assert.deepEqual(labels(tabItems(list[0], { agent: fake.agents[0], list })),
    ["Rename…", "Move right", "Open terminal", "Show panes", "Split right", "Split down", "Put away", "Close tab"]);
  assert.deepEqual(labels(tabItems(list[1], { list, spaceId: "sp1" })), ["Open", "Rename…", "Move left", "Put away", "Close tab"]);
  assert.deepEqual(labels(spaceItems(fake.spaces[0])), ["New tab here", "Rename…", "Close workspace…"]);
});

test("right-click on a pill opens the menu at the pointer; a menu row runs its command and closes", () => {
  const el = new TopView(fakeCx()).render(fakeCx());
  let stopped = false;
  byId(el, "tab-tb2")._h.on_mouse_down_right?.({}, { stop_propagation: () => { stopped = true; } });
  const o = log.overlays.at(-1);
  assert.ok(stopped);
  assert.equal(o.kind, "popup"); assert.equal(o.at, "pointer"); assert.equal(o.props.view, "menu");
  fake.props = { view: "menu", items: o.props.items, overlay: "o1" };
  const m = new MenuView(fakeCx()).render(fakeCx());
  const i = o.props.items.findIndex((x) => x.label === "Close tab");
  byId(m, `menu-${i}`)._h.on_click({}, fakeCx());
  assert.deepEqual(log.closed, ["o1"]);
  assert.deepEqual(log.run.at(-1), ["tab.close", { id: "tb2" }]);
});

test("a split tab shows its panes; one pane with an agent shows the chat", () => {
  assert.equal(screenFor(tab("tb1", 1)), "chat");
  assert.equal(screenFor(tab("tb1", 1, { panes: 2 })), "tab");
  act({ act: "panes" });
  assert.equal(fake.shared["flockbook.screen"], "tab");
});
