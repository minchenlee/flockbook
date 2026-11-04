import { test, beforeEach } from "node:test";
import assert from "node:assert/strict";
import { fake, log, reset } from "./stub/herdr.mjs";
import { textOf, byId } from "./stub/ui.mjs";
import { fakeCx } from "./fake-cx.mjs";
import { agent, space } from "./fixtures.mjs";
import { SideView, tabMarks, monogram } from "../mods/flockbook/side.js";
import { NeedsView } from "../mods/flockbook/needs.js";
import { KEY_AGENT } from "../mods/flockbook/state.js";

const tab = (id, status) => ({ id, status });

beforeEach(() => {
  reset();
  fake.spaces = [space("sp1", { name: "flock book", tabs: [tab("a", "working"), tab("b", null)] }),
    space("sp2", { name: "api", active: false, tabs: [1, 2, 3, 4, 5, 6].map((i) => tab(`x${i}`, "done")) })];
  fake.agents = [agent("t1", "blocked", { tab_name: "fix login" }), agent("t2", "working"), agent("t3", "blocked", { space: "sp2", space_name: "api" })];
});

test("tab marks: up to 5 shown, else 3 and +N; monograms", () => {
  assert.equal(tabMarks([tab(1, "done"), tab(2, null)]).shown.length, 1);
  assert.deepEqual([tabMarks(Array(6).fill(tab(1, "done"))).shown.length, tabMarks(Array(6).fill(tab(1, "done"))).more], [3, 3]);
  assert.equal(monogram("flock book"), "FB");
  assert.equal(monogram("api"), "AP");
});

test("side: Needs you badge, one row per space, clicks run herdr commands", () => {
  const el = new SideView(fakeCx()).render(fakeCx());
  assert.match(textOf(byId(el, "side-needs")), /Needs you 2/);
  assert.match(textOf(byId(el, "space-sp2")), /\+3/);
  byId(el, "space-sp2")._h.on_click({}, fakeCx());
  assert.deepEqual(log.run.slice(-2), [["space.select", { id: "sp2" }], ["layout.select", { stack: "screen", child: "chat" }]],
    "a workspace click leaves Needs you for that workspace's chat");
  assert.equal(fake.shared["flockbook.agent"], null, "the chat starts at that workspace's first agent");
  byId(el, "space-new")._h.on_click({}, fakeCx());
  assert.deepEqual(log.run.at(-1), ["space.new", undefined]);
  byId(el, "side-needs")._h.on_click({}, fakeCx());
  assert.deepEqual(log.run.at(-1), ["layout.select", { stack: "screen", child: "needs" }]);
});

test("needs: blocked agents of every space; pick one; open its chat", () => {
  const v = new NeedsView(fakeCx());
  let el = v.render(fakeCx());
  assert.match(textOf(byId(el, "needs-list")), /Needs you · 2/);
  assert.ok(byId(el, "need-t1") && byId(el, "need-t3") && !byId(el, "need-t2"));
  assert.match(textOf(byId(el, "needs-detail")), /fix login/);
  const cx = fakeCx();
  byId(el, "need-t3")._h.on_click({}, cx);
  assert.equal(cx.notified, 1);
  el = v.render(fakeCx());
  byId(el, "needs-open")._h.on_click({}, fakeCx());
  assert.equal(fake.shared[KEY_AGENT], "t3");
});

test("needs: nothing waits, All clear", () => {
  fake.agents = [agent("t2", "working")];
  assert.match(textOf(new NeedsView(fakeCx()).render(fakeCx())), /All clear/);
});
