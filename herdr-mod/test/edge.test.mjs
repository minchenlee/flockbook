import { test } from "node:test";
import assert from "node:assert/strict";
import { fake, log, reset } from "./stub/herdr.mjs";
import { fakeCx } from "./fake-cx.mjs";
import { peekAtEdge, endPeek, peekClosed, toggleSidebar, sidebarShown } from "../mods/flockbook/state.js";

test("the pointer at the left edge floats the sidebar over the content; leaving closes it", () => {
  reset(); fake.shared["flockbook.sidebar"] = false;
  peekAtEdge({ position: { x: 40, y: 100 } });
  assert.equal(log.overlays.length, 0, "not at the edge");
  peekAtEdge({ position: { x: 2, y: 100 } });
  const o = log.overlays.at(-1);
  assert.equal(o.kind, "popup"); assert.equal(o.frame, "none");
  assert.deepEqual(o.props, { view: "side", peek: true });
  assert.equal(sidebarShown(), false, "the layout's sidebar stays hidden: the content does not move");
  assert.equal(log.run.length, 0);
  peekAtEdge({ position: { x: 1, y: 100 } });
  assert.equal(log.overlays.length, 1, "one panel at a time");
  endPeek();
  assert.deepEqual(log.closed, ["o1"]);
  peekAtEdge({ position: { x: 0, y: 5 } });
  assert.equal(log.overlays.length, 2, "opens again after it closed");
});

test("a panel closed by Esc or a click beside it can open again", () => {
  reset(); fake.shared["flockbook.sidebar"] = false;
  peekAtEdge({ position: { x: 0, y: 5 } });
  peekClosed("o1");   // the panel's close(): Herdr closed it
  peekAtEdge({ position: { x: 0, y: 5 } });
  assert.equal(log.overlays.length, 2);
});

test("with the sidebar kept open there is no panel; the button closes a panel", () => {
  reset(); fake.shared["flockbook.sidebar"] = true;
  peekAtEdge({ position: { x: 0, y: 5 } });
  assert.equal(log.overlays.length, 0);
  fake.shared["flockbook.sidebar"] = false;
  peekAtEdge({ position: { x: 0, y: 5 } });
  toggleSidebar();
  assert.deepEqual(log.closed, ["o1"]);
  assert.equal(sidebarShown(), true);
});

test("the panel's side view closes itself when the pointer leaves it", async () => {
  reset(); fake.shared["flockbook.sidebar"] = false;
  fake.props = { view: "side", peek: true, overlay: "o9" };
  const { SideView } = await import("../mods/flockbook/side.js");
  const v = new SideView(fakeCx());
  const el = v.render(fakeCx());
  assert.ok(el._h.on_hover, "watches the pointer");
  el._h.on_hover(false);
  assert.deepEqual(log.closed, ["o9"]);
  fake.props = undefined;
});
