import { test } from "node:test";
import assert from "node:assert/strict";
import { fake, log, reset } from "./stub/herdr.mjs";
import { peekAtEdge, endPeek, toggleSidebar, sidebarShown } from "../mods/flockbook/state.js";

test("the pointer at the left edge peeks the sidebar; leaving closes it; the button keeps it", () => {
  reset(); fake.shared["flockbook.sidebar"] = false;
  peekAtEdge({ position: { x: 40, y: 100 } });
  assert.equal(sidebarShown(), false, "not at the edge");
  peekAtEdge({ position: { x: 2, y: 100 } });
  assert.equal(sidebarShown(), true);
  assert.deepEqual(log.run.at(-1), ["layout.show", { id: "sidebar" }]);
  endPeek();
  assert.equal(sidebarShown(), false);
  assert.deepEqual(log.run.at(-1), ["layout.hide", { id: "sidebar" }]);
  peekAtEdge({ position: { x: 0, y: 5 } });
  toggleSidebar(); toggleSidebar();   // hide, then open with the button: a kept sidebar
  const n = log.run.length;
  endPeek();
  assert.equal(log.run.length, n, "leaving a kept sidebar does nothing");
  assert.equal(sidebarShown(), true);
});
