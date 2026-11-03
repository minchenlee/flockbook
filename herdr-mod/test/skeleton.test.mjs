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
  assert.deepEqual(views.sort(), ["chat", "needs", "overview", "side", "top"]);
  assert.deepEqual(blocks(layout).filter((b) => !b.content.mod).map((b) => [b.id, b.content.tab]), [["tab", "active"]]);
});

test("keys.json binds only commands that exist, in the flockbook layout's section only", () => {
  const keys = JSON.parse(readFileSync(new URL("../keys.json", import.meta.url)));
  // Not mod:flockbook: with it, no key worked at all.
  assert.deepEqual(Object.keys(keys), ["layout:flockbook"]);
  const mine = ["terminal", "needs", "move-left", "move-right", "sidebar", "overview"];
  for (const [key, action] of Object.entries(keys["layout:flockbook"])) {
    if (action.startsWith("flockbook.")) assert.ok(mine.includes(action.slice(10)), `${key}: ${action}`);
  }
});

test("each view renders, and an unknown view says so", () => {
  for (const view of ["top", "side", "chat", "needs", "overview"]) {
    fake.props = { view }; const m = new Flockbook(); m.init(undefined, fakeCx());
    assert.ok(m.render(fakeCx()), view);
  }
  fake.props = { view: "nope" }; const m = new Flockbook(); m.init(undefined, fakeCx());
  assert.match(textOf(m.render(fakeCx())), /unknown view "nope"/);
});
