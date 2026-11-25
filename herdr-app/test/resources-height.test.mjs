// The Resources window is one height (owner 11-25): from under the bar (TOP) to GAP above the chat input, with
// any list (empty, short, long, every group folded). A drawer as before.
import { test, beforeEach } from "node:test";
import assert from "node:assert/strict";
import { fake, log, reset } from "./stub/herdr.mjs";
import { fakeCx } from "./fake-cx.mjs";
import { agent, space } from "./fixtures.mjs";
import * as R from "../mods/flockbook/resources.js";
import { forgetCommands } from "../mods/flockbook/sidecar.js";

const res = (id, kind, last_ts, target = `https://x.com/${id}`) => ({ id, kind, target, title: id, detail: "x.com", row: `r-${id}`, first_ts: last_ts, last_ts, mentions: 1, exists: true });

beforeEach(() => {
  reset(); forgetCommands(); R.forgetResources(); R.forgetCache();
  fake.spaces = [space("sp1", { tabs: [{ id: "tb-t1", index: 1, name: "1", active: true, status: "idle", panes: 1, settled: false }] })];
  fake.agents = [agent("t1", "idle", { tab: "tb-t1" })];
});

test("placement: the same height for any count, from y 60 to 12 px above the input", () => {
  const full = 900 - 150 - 12 - 60;
  for (const [count, groups] of [[0, 0], [1, 1], [3, 2], [40, 6], [500, 9]]) {
    const p = R.placement(1470, 900, count, groups, 1470, 150);
    assert.equal(p.height, full, `${count} rows, ${groups} titles`);
    assert.equal(p.at.y + p.height, 900 - 150 - 12);
  }
  assert.equal(R.placement(1470, 900, 0, 0).height, 900 - R.INPUT_GUESS - 12 - 60, "the input not measured yet: INPUT_GUESS");
  const d = R.placement(700, 900, 0, 0, 700, 150);
  assert.deepEqual([d.drawer, d.at.y, d.height], [true, 0, 900 - 150 - 12], "a drawer as before");
});

test("the open window keeps its size when its list changes: empty, more items, every group folded, a group shown in full", () => {
  fake.shared[R.KEY_WIN_W] = 1400; fake.shared["flockbook.window.height"] = 900; fake.shared[R.KEY_INPUT_H] = 150;
  R.putData({ terminal: "t1", cursor: "c", items: [], at: 1 });
  const empty = R.placeNow();
  R.putData({ terminal: "t1", cursor: "c", items: Array.from({ length: 80 }, (_, i) => res(`i${i}`, i % 2 ? "link" : "pr", 100 - i)), at: 1 });
  assert.equal(R.placeNow().height, empty.height, "many");
  assert.equal(R.placeNow(["links"]).height, empty.height, "one shown in full");
  fake.shared[R.KEY_COLLAPSED] = ["links", "github"];
  assert.equal(R.placeNow().height, empty.height, "all folded");
  // Open: a tick after the list changed does not open it again.
  fake.shared[R.KEY_COLLAPSED] = [];
  fake.props = { view: "resources", overlay: "o7" };
  fake.shared[R.KEY_OPEN] = "o7"; fake.shared[R.KEY_RUN] = R.RUN; fake.shared[R.KEY_FRAME] = empty;
  const v = new R.ResourcesView(fakeCx());
  const n = log.overlays.length;
  R.putData({ terminal: "t1", cursor: "c", items: [res("a", "link", 1)], at: 1 });
  v.tick(fakeCx());
  assert.equal(log.overlays.length, n, "not opened again");
});
