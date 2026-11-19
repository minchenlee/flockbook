import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import Flockbook from "../mods/flockbook/main.js";
import { textOf } from "./stub/ui.mjs";
import { fakeCx } from "./fake-cx.mjs";
import { fake } from "./stub/herdr.mjs";

const layout = JSON.parse(readFileSync(new URL("../layouts/flockbook.json", import.meta.url)));
function blocks(n, out = []) { if (n.type === "block") out.push(n); (n.children || []).forEach((c) => blocks(c, out)); return out; }

test("every layout block shows the flockbook mod with a known view, or the shown tab", () => {
  const mods = blocks(layout).filter((b) => b.content.mod);
  const views = mods.map((b) => { assert.equal(b.content.mod, "flockbook"); return b.content.props.view; });
  assert.deepEqual(views.sort(), ["chat", "needs", "overview", "side", "top"], "settings is a dialog, not a block");
  assert.deepEqual(blocks(layout).filter((b) => !b.content.mod).map((b) => [b.id, b.content.tab]), [["tab-panes", "active"]]);
});

test("only the top bar moves the window: it sits above the screens, and no screen block has window_drag (#16)", () => {
  // A drag that selects chat text must not move the window. A press stop above the text would keep the window
  // still but makes the text unselectable (Herdr 0.1.13+), so the screens must not be window_drag at all.
  const drag = []; const walk = (n) => { if (n.window_drag) drag.push(n.id); (n.children || []).forEach(walk); };
  walk(layout);
  assert.deepEqual(drag, ["top"]);
  const work = layout.children.find((c) => c.id === "work");
  assert.deepEqual(work.children.map((c) => c.id).slice(0, 2), ["top", "screen"], "the bar is above the screen stack");
});

test("keys.json binds only commands that exist, in the flockbook layout's section only", () => {
  const keys = JSON.parse(readFileSync(new URL("../keys.json", import.meta.url)));
  // Not mod:flockbook: with it, no key worked at all.
  assert.deepEqual(Object.keys(keys), ["layout:flockbook"]);
  const mine = ["terminal", "needs", "move-left", "move-right", "sidebar", "overview", "settings", "text-smaller", "text-larger", "text-default"];
  for (const [key, action] of Object.entries(keys["layout:flockbook"])) {
    if (action.startsWith("flockbook.")) assert.ok(mine.includes(action.slice(10)), `${key}: ${action}`);
  }
});

test("each view renders, and an unknown view says so", () => {
  for (const view of ["top", "side", "chat", "needs", "overview", "settings"]) {
    fake.props = { view }; const m = new Flockbook(); m.init(undefined, fakeCx());
    assert.ok(m.render(fakeCx()), view);
  }
  fake.props = { view: "nope" }; const m = new Flockbook(); m.init(undefined, fakeCx());
  assert.match(textOf(m.render(fakeCx())), /unknown view "nope"/);
});

test("the screens under the top bar note the whole window's height, bar included (the floating sidebar uses it)", async () => {
  const { reset, fake: f } = await import("./stub/herdr.mjs");
  const { BAR_H } = await import("../mods/flockbook/top.js");
  const { KEY_WIN_H } = await import("../mods/flockbook/state.js");
  for (const view of ["chat", "needs", "overview"]) {
    reset(); f.props = { view }; const m = new Flockbook(); const cx = fakeCx(); m.init(undefined, cx);
    const root = m.render(cx);
    const find = (el) => { if (!el || typeof el !== "object") return null; if (el._h?.on_size_change) return el;
      for (const k of el.kids || []) { const r = find(k); if (r) return r; } return null; };
    const el = find(root);
    assert.ok(el, `${view} listens for its size`);
    el._h.on_size_change({ width: 900, height: 764 }, cx);
    assert.equal(f.shared[KEY_WIN_H], 764 + BAR_H, view);
  }
});
