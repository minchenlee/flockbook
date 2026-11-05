// Drag a workspace row in the sidebar up or down to change the order (space.move), like a tab in the top bar.
import { test, beforeEach } from "node:test";
import assert from "node:assert/strict";
import { fake, log, reset } from "./stub/herdr.mjs";
import { byId } from "./stub/ui.mjs";
import { fakeCx } from "./fake-cx.mjs";
import { space } from "./fixtures.mjs";
import { SideView } from "../mods/flockbook/side.js";

beforeEach(() => {
  reset();
  fake.props = undefined;
  fake.spaces = [space("sp1", { index: 1, name: "app" }), space("sp2", { index: 2, name: "api", active: false }),
    space("sp3", { index: 3, name: "docs", active: false }), space("sp4", { index: 4, name: "web", active: false })];
});

// Rows 34 px high, 1 px apart, from y 200.
const Y = { sp1: 200, sp2: 235, sp3: 270, sp4: 305 };
const at = (id, dy) => ({ button: "left", click_count: 1, modifiers: {}, position: { x: 60, y: Y[id] + dy },
  local_position: { x: 50, y: dy }, bounds: { x: 8, y: Y[id], width: 244, height: 34 } });
const row = (el, id) => byId(el, `space-${id}`);
const cx = () => ({ ...fakeCx(), stop_propagation() {} });
const press = (el, id, dy = 10) => row(el, id)._h.on_mouse_down(at(id, dy), cx());
const move = (el, id, dy) => row(el, id)._h.on_mouse_move(at(id, dy), cx());
const release = (el, id, dy) => byId(el, "spaces")._h.on_mouse_up(at(id, dy), cx());
const render = () => new SideView(fakeCx()).render(fakeCx());

test("drag a workspace down: the line shows below the row under the pointer, release moves it there once", () => {
  let el = render();
  press(el, "sp1"); move(el, "sp1", 16); move(el, "sp3", 25);
  el = render();
  const line = byId(row(el, "sp3"), "space-drop-line");
  assert.ok(line, "the line is on sp3");
  assert.ok(line._a.bottom !== undefined && line._a.top === undefined, "below it");
  assert.equal(byId(el, "space-drop-line"), line, "one line only");
  assert.ok(row(el, "sp1")._a.opacity < 1, "the dragged row is dimmed");
  release(el, "sp3", 25);
  assert.deepEqual(log.run, [["space.move", { id: "sp1", index: 3 }]]);
  assert.equal(byId(render(), "space-drop-line"), null, "the line goes after the drop");
});

test("drag a workspace up: above the row under the pointer", () => {
  let el = render();
  press(el, "sp4"); move(el, "sp4", 4); move(el, "sp2", 8);
  el = render();
  assert.ok(byId(row(el, "sp2"), "space-drop-line")._a.top !== undefined, "above sp2");
  release(el, "sp2", 8);
  assert.deepEqual(log.run, [["space.move", { id: "sp4", index: 2 }]]);
});

test("a press that moves less than 4 px is a click: the workspace is shown, nothing moves", () => {
  const el = render();
  press(el, "sp2", 10); move(el, "sp2", 13);
  assert.equal(byId(render(), "space-drop-line"), null);
  row(el, "sp2")._h.on_click({}, cx());
  release(el, "sp2", 13);
  assert.deepEqual(log.run.map((r) => r[0]), ["space.select", "layout.select"]);
});

test("a drag back to its own place moves nothing and does not select the workspace", () => {
  const el = render();
  press(el, "sp2"); move(el, "sp2", 30); move(el, "sp1", 30);
  assert.equal(byId(render(), "space-drop-line"), null, "below sp1 is where sp2 already is");
  row(el, "sp2")._h.on_click({}, cx());
  release(el, "sp1", 30);
  assert.deepEqual(log.run, []);
});

test("leaving the list, another button, or a press elsewhere cancels the drag", () => {
  let el = render();
  press(el, "sp1"); move(el, "sp1", 16); move(el, "sp3", 25);
  byId(el, "spaces")._h.on_hover(false, cx());
  el = render();
  assert.equal(byId(el, "space-drop-line"), null);
  assert.equal(row(el, "sp1")._a.opacity, undefined);
  release(el, "sp3", 25);
  move(el, "sp3", 25);
  assert.equal(byId(render(), "space-drop-line"), null, "moving over it again with no press starts nothing");

  press(el, "sp1"); move(el, "sp1", 16); move(el, "sp3", 25);
  row(el, "sp3")._h.on_mouse_down_right(at("sp3", 25), cx());
  assert.equal(log.overlays.length, 0, "the right press only cancels; no menu during a drag");
  release(el, "sp3", 25);

  press(el, "sp1"); move(el, "sp1", 16); move(el, "sp3", 25);
  byId(el, "spaces")._h.on_mouse_down_out(at("sp3", 25), cx());
  release(el, "sp3", 25);
  assert.deepEqual(log.run, []);
});

test("a release between rows ends the press; a drag released there drops at the last place shown", () => {
  let el = render();
  press(el, "sp1");
  byId(el, "spaces")._h.on_mouse_up(at("sp1", 34.5), cx());
  move(el, "sp1", 16); move(el, "sp3", 25);
  assert.equal(byId(render(), "space-drop-line"), null, "no drag without a press");
  el = render();
  press(el, "sp1"); move(el, "sp1", 16); move(el, "sp3", 25);
  byId(el, "spaces")._h.on_mouse_up(at("sp3", 34.5), cx());
  assert.deepEqual(log.run, [["space.move", { id: "sp1", index: 3 }]]);
});

test("the right-click menu and a click still work with no drag", () => {
  const el = render();
  row(el, "sp3")._h.on_mouse_down_right(at("sp3", 10), cx());
  assert.equal(log.overlays.length, 1, "the workspace menu opens");
  row(el, "sp3")._h.on_click({}, cx());
  assert.deepEqual(log.run.at(-2), ["space.select", { id: "sp3" }]);
});

test("the floating sidebar drags the same way", () => {
  fake.props = { view: "side", peek: true, overlay: "o9" };
  let el = render();
  press(el, "sp1"); move(el, "sp1", 16); move(el, "sp2", 25);
  el = render();
  assert.ok(byId(row(el, "sp2"), "space-drop-line"));
  release(el, "sp2", 25);
  assert.deepEqual(log.run, [["space.move", { id: "sp1", index: 2 }]]);
});

test("the tab drag and the workspace drag do not mix", async () => {
  const { TopView } = await import("../mods/flockbook/top.js");
  fake.spaces[0].tabs = [{ id: "t1", index: 1, name: "1", active: true, status: null }, { id: "t2", index: 2, name: "2", active: false, status: null }];
  fake.layout = { id: "root", children: [{ id: "sidebar", hidden: false }, { id: "screen", active: "chat" }] };
  let el = render();
  press(el, "sp1"); move(el, "sp1", 16); move(el, "sp3", 25);
  const bar = new TopView(fakeCx()).render(fakeCx());
  assert.equal(byId(bar, "drop-line"), null, "no line in the top bar");
  byId(bar, "tabs")._h.on_mouse_up({ position: { x: 0, y: 0 } }, cx());
  assert.deepEqual(log.run, [], "a release on the tab bar does not drop the workspace or move a tab");
});
