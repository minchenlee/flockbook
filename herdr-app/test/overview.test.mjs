import { test, beforeEach } from "node:test";
import assert from "node:assert/strict";
import { fake, log, reset } from "./stub/herdr.mjs";
import { textOf, byId } from "./stub/ui.mjs";
import { fakeCx } from "./fake-cx.mjs";
import { agent, space } from "./fixtures.mjs";
import { OverviewView, groups, allTabs, ago, greeting, KEY_GROUP } from "../mods/flockbook/overview.js";

const tab = (id, index, extra = {}) => ({ id, index, name: String(index), named: false, active: false, status: null, panes: 1, zoomed: false, settled: false, settled_at: null, changed_at: null, working_since: null, ...extra });

beforeEach(() => {
  reset();
  fake.spaces = [
    space("sp1", { name: "app", tabs: [tab("tb-t1", 1, { status: "working", name: "api", named: true }), tab("tb-t2", 2, { status: "done" }), tab("tb-x", 3, { settled: true })] }),
    space("sp2", { name: "docs", active: false, tabs: [tab("tb-t3", 1, { status: "blocked" }), tab("tb-sh", 2)] }),
  ];
  fake.agents = [agent("t1", "working"), agent("t2", "done"), agent("t3", "blocked", { space: "sp2" })];
});

test("every tab of every space, except settled ones", () => {
  assert.deepEqual(allTabs().map((it) => it.tab.id), ["tb-t1", "tb-t2", "tb-t3", "tb-sh"]);
});

test("grouped by status: needs you, working, done, then resting; or by space", () => {
  assert.deepEqual(groups(allTabs(), "status").map((g) => [g.title, g.items.length]), [["Needs you", 1], ["Working", 1], ["Done", 1], ["Resting", 1]]);
  assert.deepEqual(groups(allTabs(), "space").map((g) => [g.title, g.items.length]), [["app", 2], ["docs", 2]]);
});

test("header counts, cards, and a click opens that tab in its space", () => {
  const v = new OverviewView(fakeCx());
  const el = v.render(fakeCx());
  assert.match(textOf(el), /4 tabs · 1 working · 1 need you/);
  assert.match(textOf(byId(el, "ov-tb-t1")), /api/);
  byId(el, "ov-tb-t3")._h.on_click({}, fakeCx());
  assert.deepEqual(log.run, [["space.select", { id: "sp2" }], ["tab.select", { id: "tb-t3" }], ["layout.select", { stack: "screen", child: "chat" }]]);
  byId(el, "group-space")._h.on_click({}, fakeCx());
  assert.equal(fake.shared[KEY_GROUP], "space");
});

test("ago and greeting", () => {
  const now = 10 * 86400000;
  assert.equal(ago(now - 20000, now), "now");
  assert.equal(ago(now - 13 * 60000, now), "13m");
  assert.equal(ago(now - 5 * 3600000, now), "5h");
  assert.equal(ago(now - 2 * 86400000, now), "2d");
  assert.equal(ago(null, now), "");
  assert.equal(greeting(9), "Good morning"); assert.equal(greeting(15), "Good afternoon"); assert.equal(greeting(21), "Good evening");
});
