import { test } from "node:test";
import assert from "node:assert/strict";
import { fake, log, reset } from "./stub/herdr.mjs";
import { fakeCx } from "./fake-cx.mjs";
import { peekAtEdge, endPeek, peekClosed, toggleSidebar, sidebarShown, noteWindowHeight } from "../mods/flockbook/state.js";

test("the pointer at the left edge floats the sidebar over the content; leaving closes it", () => {
  reset(); fake.shared["flockbook.sidebar"] = false;
  peekAtEdge({ position: { x: 40, y: 100 } });
  assert.equal(log.overlays.length, 0, "not at the edge");
  peekAtEdge({ position: { x: 2, y: 100 } });
  const o = log.overlays.at(-1);
  assert.equal(o.kind, "window", "a popup ignores the height"); assert.equal(o.frame, "none");
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

test("the panel is as tall as the window, less a gap above and below", () => {
  reset(); fake.shared["flockbook.sidebar"] = false;
  noteWindowHeight({ width: 900, height: 820 });
  peekAtEdge({ position: { x: 0, y: 5 } });
  assert.equal(log.overlays.at(-1).height, 820 - 16);
});

test("the panel slides in: first drawn left of its spot, then moved to it with a spring", async () => {
  reset(); fake.shared["flockbook.sidebar"] = false;
  fake.props = { view: "side", peek: true, overlay: "o9" };
  const { SideView } = await import("../mods/flockbook/side.js");
  const cx = fakeCx(); const v = new SideView(cx);
  const first = v.render(cx);
  assert.ok(first._a.left < 0, "starts out of view");
  cx.timers.filter((x) => x.once).forEach((x) => x.f(cx));
  const next = v.render(cx);
  assert.equal(next._a.left, 0);
  assert.ok(next._a.spring);
  fake.props = undefined;
});

test("the sidebar button is on the same spot in the panel as in the docked sidebar", async () => {
  const { SideView } = await import("../mods/flockbook/side.js");
  const { byId } = await import("./stub/ui.mjs");
  const { BAR_H, lightsRoom } = await import("../mods/flockbook/top.js");
  const spot = (peek) => {
    reset(); fake.shared["flockbook.sidebar"] = !peek;
    fake.props = peek ? { view: "side", peek: true, overlay: "o9" } : { view: "side" };
    const el = new SideView(fakeCx()).render(fakeCx());
    const header = el.kids.find((k) => byId(k, "side-toggle"));
    // The panel is drawn at (PEEK_GAP, PEEK_GAP) with a 1 px border; the sidebar at (0, 0).
    const at = peek ? 8 + 1 : 0;
    return { x: at + 8 + (header._a.ml ?? 0) + header._a.pl + 14, y: at + header._a.h / 2 };
  };
  const docked = spot(false), floating = spot(true);
  fake.props = undefined;
  assert.deepEqual(floating, docked);
  assert.deepEqual(docked, { x: lightsRoom() + 14, y: BAR_H / 2 }, "where the top bar has it while the sidebar is hidden");
});
