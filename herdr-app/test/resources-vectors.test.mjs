import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import * as R from "../mods/flockbook/resources.js";

// The shared Resources vectors (Tests/Fixtures/resources/vectors.json): the Mac app puts a row in the same group.
const v = JSON.parse(readFileSync(new URL("../../Tests/Fixtures/resources/vectors.json", import.meta.url), "utf8"));

test("each kind and target goes to the group the shared vectors give", () => {
  assert.ok(v.groups.length > 10);
  for (const [kind, target, group] of v.groups) assert.equal(R.groupOf({ kind, target }), group, `${kind} ${target}`);
});

test("the groups are in the shared order", () => {
  const order = [...new Set(v.groups.map((g) => g[2]))];
  assert.deepEqual(R.GROUPS.map((g) => g.id).filter((g) => order.includes(g)), order);
});
