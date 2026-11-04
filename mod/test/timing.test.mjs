import { test } from "node:test";
import assert from "node:assert/strict";
import { fake, reset } from "./stub/herdr.mjs";
import { loadTiming, record, flush } from "../mods/flockbook/timing.js";

const capture = (fn) => { const out = []; const log = console.log; console.log = (s) => out.push(s); try { fn(); } finally { console.log = log; } return out; };

test("timing is off without the flag file, and writes one line a second with it", async () => {
  reset();
  assert.equal(await loadTiming(), false);
  assert.deepEqual(capture(() => { record("chat.render", 3); flush(); }), []);
  fake.readText = (p) => (p.endsWith("/flockbook/timing") ? "" : null);
  assert.equal(await loadTiming(), true);
  const out = capture(() => { record("chat.render", 2); record("chat.render", 4); record("chat.rows", 1, 12); flush(); flush(); });
  assert.equal(out.length, 1);
  assert.match(out[0], /\[flockbook-timing\] .*chat\.render n=2 avg=3\.00 max=4\.00 \| chat\.rows n=1 avg=1\.00 max=1\.00 x=12/);
  fake.readText = null; await loadTiming();
});
