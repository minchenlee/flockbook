// Resources with a long conversation (hg-res-perf, owner 11-25: 761 items, "超級卡"): a cache per agent in memory,
// a small flockbook.res.data, at most ROW_LIMIT rows a group ("Show N more"), groups that fold.
import { test, beforeEach } from "node:test";
import assert from "node:assert/strict";
import { fake, log, reset } from "./stub/herdr.mjs";
import { textOf, byId } from "./stub/ui.mjs";
import { fakeCx } from "./fake-cx.mjs";
import { agent, space } from "./fixtures.mjs";
import * as R from "../mods/flockbook/resources.js";
import { forgetCommands } from "../mods/flockbook/sidecar.js";

const jl = (...lines) => ({ code: 0, stdout: lines.map((l) => JSON.stringify(l)).join("\n") + "\n", stderr: "" });
const header = (cmd) => ({ type: "header", protocol: 1, sidecar: "0.1.0", cmd });
const end = (cursor, full) => ({ type: "end", ok: true, ms: 1, cursor, full });

// The owner's conversation: 761 items (436 docs, 40 artifacts, 60 PRs, 75 commands, 150 links).
export const KINDS = [["doc", 436], ["artifact", 40], ["pr", 60], ["cmd", 75], ["link", 150]];
export function long(prefix = "") {
  const out = []; let n = 0;
  for (const [kind, c] of KINDS) for (let i = 0; i < c; i++, n++) {
    out.push({ type: "res", id: `${prefix}id${n}`, kind, target: kind === "doc" ? `/home/u/p/docs/f${n}.md` : `https://example.com/a/${n}`,
      title: `Title ${n}`, detail: "example.com", row: `r-${n}`, first_ts: 1e12 + n, last_ts: 1e12 + n * 1000, mentions: 1, exists: true });
  }
  return out;
}

// A helper with the command. `lists[t]`: that agent's items; a --since ask gets `delta[t]` (default none).
function helper(lists, delta = {}) {
  const asked = [];
  fake.exec = (_cmd, args) => {
    if (args[0] === "version") return jl(header("version"), { type: "version", commands: ["version", "feed", "resources"] }, end());
    if (args[0] === "resources") {
      asked.push(args);
      const t = args[args.indexOf("--terminal") + 1];
      const since = args.includes("--since");
      return jl(header("resources"), ...(since ? delta[t] ?? [] : lists[t] ?? []), end(`c-${t}`, !since));
    }
    return jl(header(args[0]), { type: "end", ok: true, ms: 1 });
  };
  return asked;
}
const showAgent = (t) => { fake.agents = [agent(t, "idle", { tab: "tb-t1" })]; };

beforeEach(() => {
  reset(); forgetCommands(); R.forgetResources(); R.forgetCache();
  fake.spaces = [space("sp1", { tabs: [{ id: "tb-t1", index: 1, name: "1", active: true, status: "idle", panes: 1, settled: false }] })];
  showAgent("t1");
});

test("flockbook.res.data is small: no items in the persisted value, the list only in memory", async () => {
  helper({ t1: long() });
  await R.pollResources(fakeCx(), true);
  const d = fake.shared[R.KEY_DATA];
  assert.equal(d.terminal, "t1");
  assert.equal(d.items, undefined, "no items in shared");
  assert.equal(d.count, 761);
  assert.ok(JSON.stringify(d).length < 512, `${JSON.stringify(d).length} bytes`);
  assert.equal(R.dataFor("t1").items.length, 761, "the list is in memory");
  for (const [k, v] of log.shared) assert.ok(JSON.stringify(v ?? null).length < 1024, `${k}: ${JSON.stringify(v).length} bytes`);
});

test("back to an agent seen before: its list shows at once, and the ask is --since its cursor (no full read)", async () => {
  const asked = helper({ t1: long("a"), t2: long("b").slice(0, 5) });
  await R.pollResources(fakeCx(), true);
  showAgent("t2");
  await R.pollResources(fakeCx(), true);
  assert.equal(R.dataFor("t2").items.length, 5);
  showAgent("t1");
  assert.equal(R.dataFor("t1")?.items.length, 761, "shown before any ask");
  await R.pollResources(fakeCx(), true);
  const last = asked.at(-1);
  assert.equal(last[last.indexOf("--terminal") + 1], "t1");
  assert.equal(last[last.indexOf("--since") + 1], "c-t1", "asked from its cursor");
  assert.equal(R.dataFor("t1").items.length, 761, "a delta with nothing new keeps the list");
  assert.equal(asked.filter((a) => !a.includes("--since")).length, 2, "one full read per agent");
});

test("a delta with nothing new writes nothing and does not redraw", async () => {
  helper({ t1: long() });
  await R.pollResources(fakeCx(), true);
  const n = log.shared.filter(([k]) => k === R.KEY_DATA).length;
  const cx = fakeCx();
  await R.pollResources(cx, true);
  assert.equal(log.shared.filter(([k]) => k === R.KEY_DATA).length, n);
  assert.equal(cx.notified, 0);
});

test("the same full answer again (the window opened) writes nothing; a new item does", async () => {
  const list = long();
  helper({ t1: list });
  await R.pollResources(fakeCx(), true, true);
  const n = log.shared.filter(([k]) => k === R.KEY_DATA).length;
  await R.pollResources(fakeCx(), true, true);
  assert.equal(log.shared.filter(([k]) => k === R.KEY_DATA).length, n, "nothing changed");
  list.push({ ...list[0], id: "new", last_ts: 2e12 });
  const cx = fakeCx();
  await R.pollResources(cx, true, true);
  assert.equal(log.shared.filter(([k]) => k === R.KEY_DATA).length, n + 1);
  assert.equal(fake.shared[R.KEY_DATA].count, 762);
  assert.ok(cx.notified > 0);
});

test("after a restart (the cache is empty) the first ask is a full read, also with a cursor left in shared", async () => {
  // What a restart leaves: the small value, or a trial build's big one with items.
  fake.shared[R.KEY_DATA] = { terminal: "t1", cursor: "c-old", count: 3, at: 1, items: [{ id: "stale", kind: "link", last_ts: 1 }] };
  const asked = helper({ t1: long().slice(0, 4) });
  assert.equal(R.dataFor("t1"), null, "the old items in shared are not shown");
  await R.pollResources(fakeCx());
  assert.ok(!asked[0].includes("--since"), "no cursor without the list it belongs to");
  assert.equal(R.dataFor("t1").items.length, 4);
  assert.equal(fake.shared[R.KEY_DATA].items, undefined, "the big value is replaced by the small one");
});

test("the cache keeps the last CACHE_MAX agents asked about (one asked about again is kept)", async () => {
  helper(Object.fromEntries(Array.from({ length: R.CACHE_MAX + 1 }, (_, i) => [`t${i}`, long(`x${i}`).slice(0, 2)])));
  for (let i = 0; i < R.CACHE_MAX; i++) { showAgent(`t${i}`); await R.pollResources(fakeCx(), true); }
  // t0 again: now the newest.
  showAgent("t0"); await R.pollResources(fakeCx(), true);
  showAgent(`t${R.CACHE_MAX}`); await R.pollResources(fakeCx(), true);
  assert.equal(R.dataFor("t0")?.items.length, 2, "asked about again: kept");
  assert.equal(R.dataFor("t1"), null, "the one asked about longest ago went");
  assert.equal(R.dataFor("t2").items.length, 2);
  assert.equal(R.dataFor(`t${R.CACHE_MAX}`).items.length, 2);
});

test("the open window, back to an agent: its list at once, then a full read (a doc written meanwhile is there now) (Codex R1)", async () => {
  const doc = { type: "res", id: "d", kind: "doc", target: "/home/u/p/plan.md", title: "plan.md", detail: "~/p", row: "r-d", first_ts: 5, last_ts: 5, mentions: 1, exists: false };
  const lists = { t1: [doc], t2: long("b").slice(0, 2) };
  const asked = helper(lists);   // a --since ask gets nothing: the log did not change
  fake.props = { view: "resources", overlay: "o7" };
  fake.shared[R.KEY_OPEN] = "o7"; fake.shared[R.KEY_RUN] = R.RUN;
  const cx = fakeCx();
  const v = new R.ResourcesView(cx);
  await cx.settle();
  assert.equal(R.dataFor("t1").items[0].exists, false);
  showAgent("t2"); v.tick(cx); await cx.settle();
  // The file is written now.
  lists.t1 = [{ ...doc, exists: true }];
  showAgent("t1"); v.tick(cx);
  assert.equal(R.dataFor("t1")?.items.length, 1, "shown at once");
  await cx.settle();
  const last = asked.filter((a) => a.includes("t1")).at(-1);
  assert.ok(!last.includes("--since"), "a full read");
  assert.equal(R.dataFor("t1").items[0].exists, true);
});

test("a full read asked while a delta runs is done after it, not dropped (Codex R1)", async () => {
  const lists = { t1: long().slice(0, 3) };
  const asked = helper(lists);
  await R.pollResources(fakeCx(), true);
  lists.t1 = lists.t1.map((i) => ({ ...i, title: `${i.title} new` }));
  const cx = fakeCx();
  const delta = R.pollResources(cx, true);           // the bar's ask, running
  const full = R.pollResources(cx, true, true);      // the window opens now
  await Promise.all([delta, full]); await cx.settle();
  assert.deepEqual(asked.map((a) => a.includes("--since")), [false, true, false], "the full one after the delta");
  assert.match(R.dataFor("t1").items[0].title, / new$/);
  // Also when the delta ends early (the helper failed): the full one still runs after it.
  const was = fake.exec; let n = 0;
  fake.exec = (c, args) => (args[0] === "resources" && n++ === 0 ? jl(header("resources"), { type: "error", code: "read", message: "x" }, { type: "end", ok: false, ms: 1 }) : was(c, args));
  lists.t1 = lists.t1.map((i) => ({ ...i, title: "again" }));
  R.forgetCache();   // nothing to keep: the failed ask returns early
  const d2 = R.pollResources(cx, true), f2 = R.pollResources(cx, true, true);
  await Promise.all([d2, f2]);
  assert.equal(R.dataFor("t1").items[0].title, "again");
});

const open = (items, extra = {}) => {
  R.putData({ terminal: "t1", cursor: "c1", items: items.map(({ type: _t, ...r }) => r), at: 1 });
  fake.shared[R.seenKey("t1")] = 0;
  fake.props = { view: "resources", overlay: "o7", ...extra };
  fake.shared[R.KEY_OPEN] = "o7"; fake.shared[R.KEY_RUN] = R.RUN;
  fake.shared[R.KEY_WIN_W] = 1400; fake.shared["flockbook.window.height"] = 900;
  return new R.ResourcesView(fakeCx());
};
const rowsIn = (el) => (el?.kids ?? []).filter((k) => typeof k?._a?.id === "string" && /^res-(?!group|more|chev)/.test(k._a.id) && !k._a.id.startsWith("res-group"));

test("at most ROW_LIMIT rows a group, then a Show N more row; a click shows the rest, also after a refit", () => {
  assert.equal(R.ROW_LIMIT, 30);
  const v = open(long());
  fake.shared[R.KEY_FRAME] = R.placeNow();
  let el = v.render(fakeCx());
  const docs = byId(el, "res-group-docs");
  assert.equal(rowsIn(docs).length, 30);
  assert.match(textOf(byId(el, "res-group-head-docs")), /Docs\s*436/, "the count is all of them");
  assert.match(textOf(byId(el, "res-more-docs")), /Show 406 more/);
  assert.equal(byId(el, "res-more-artifacts")?._a?.id, "res-more-artifacts");
  assert.match(textOf(byId(el, "res-more-artifacts")), /Show 10 more/);
  byId(el, "res-more-docs")._h.on_click(null, fakeCx());
  el = v.render(fakeCx());
  assert.equal(rowsIn(byId(el, "res-group-docs")).length, 436);
  assert.equal(byId(el, "res-more-docs"), null);
  // Taller now (a very tall window, so the height is not at its most): opened again, the new one keeps the group shown.
  fake.shared["flockbook.window.height"] = 30000;
  fake.shared[R.KEY_FRAME] = R.placeNow();
  v.expanded = new Set(); assert.ok(R.placeNow(["docs"]).height > R.placeNow().height);
  v.expanded = new Set(["docs"]);
  v.refit();
  const spec = log.overlays.at(-1);
  assert.deepEqual(spec.props.expanded, ["docs"]);
  fake.props = { ...spec.props, overlay: "o8" };
  const again = new R.ResourcesView(fakeCx()).render(fakeCx());
  assert.equal(rowsIn(byId(again, "res-group-docs")).length, 436);
});

test("the height counts the rows drawn, not all of them", () => {
  open(long());
  fake.shared["flockbook.window.height"] = 30000;   // tall enough that the height is not at its most
  const p = R.placeNow();
  // 5 groups, each with ROW_LIMIT rows drawn: 150 rows; 5 titles and 5 "Show more" rows.
  assert.equal(p.height, R.placement(1400, 30000, 150, 10).height);
  assert.notEqual(p.height, R.placement(1400, 30000, 151, 10).height, "one row more is another height");
  assert.notEqual(p.height, R.placement(1400, 30000, 150, 9).height);
  assert.equal(R.placeNow(["docs"]).height, R.placement(1400, 30000, 150 - 30 + 436, 9).height, "docs shown in full: no Show more there");
  fake.shared["flockbook.window.height"] = 900;
  const few = long().filter((i) => i.kind === "pr").slice(0, 3);
  open(few);
  assert.equal(R.placeNow().height, R.placement(1400, 900, 3, 1).height, "a short list: as before");
});

test("a group folds: a click on its title, a chevron, the count stays, kept for every agent and a restart, the window shorter", () => {
  const v = open(long());
  let el = v.render(fakeCx());
  assert.match(byId(el, "res-chev-docs")._a.path, /chevron-down\.svg$/);
  const tall = R.placeNow().height;
  byId(el, "res-group-head-docs")._h.on_click(null, fakeCx());
  assert.deepEqual(fake.shared[R.KEY_COLLAPSED], ["docs"]);
  el = v.render(fakeCx());
  assert.equal(rowsIn(byId(el, "res-group-docs")).length, 0, "no rows");
  assert.equal(byId(el, "res-more-docs"), null);
  assert.match(textOf(byId(el, "res-group-head-docs")), /Docs\s*436/, "the count is still there");
  assert.match(byId(el, "res-chev-docs")._a.path, /chevron-right\.svg$/);
  assert.ok(R.placeNow().height < tall || R.placeNow().height === tall && tall === 900 - R.INPUT_GUESS - 12 - 60, "not taller");
  // Few rows: the fold makes it shorter for sure.
  const w = open(long().filter((i) => i.kind === "pr").slice(0, 5).concat(long().filter((i) => i.kind === "link").slice(0, 5)));
  fake.shared[R.KEY_COLLAPSED] = [];
  const h1 = R.placeNow().height;
  byId(w.render(fakeCx()), "res-group-head-github")._h.on_click(null, fakeCx());
  assert.ok(R.placeNow().height < h1, `${R.placeNow().height} < ${h1}`);
  // A new window (a restart: shared is persisted): still folded; another agent too.
  showAgent("t2");
  R.putData({ terminal: "t2", cursor: "c", items: long().filter((i) => i.kind === "pr").slice(0, 2).map(({ type: _t, ...r }) => r), at: 1 });
  const other = new R.ResourcesView(fakeCx()).render(fakeCx());
  assert.equal(rowsIn(byId(other, "res-group-github")).length, 0, "folded for another agent too");
  // A click again unfolds.
  byId(other, "res-group-head-github")._h.on_click(null, fakeCx());
  assert.deepEqual(fake.shared[R.KEY_COLLAPSED], []);
});

const count = (el) => (!el || typeof el !== "object" ? 0 : 1 + (el.kids ?? []).reduce((a, k) => a + count(k), 0));

// Measured on an Apple M2 (16 GB), node 26.13, load about 5: before this change one render of the 761 items made
// 6117 elements and took about 4.3 ms here (the stub's elements; Herdr's are dearer). Now: under 1500 elements.
test("761 items: one render makes a bounded tree, and is quick", () => {
  const v = open(long());
  assert.ok(count(v.render(fakeCx())) < 1500, `${count(v.render(fakeCx()))} elements`);
  const t0 = performance.now();
  for (let i = 0; i < 20; i++) v.render(fakeCx());
  const ms = (performance.now() - t0) / 20;
  assert.ok(ms < 15, `${ms.toFixed(2)} ms a render`);
});
