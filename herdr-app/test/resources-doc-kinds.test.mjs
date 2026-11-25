// Docs in 4 groups by the file's extension (owner 11-25): Markdown, Web pages, PDFs, Images; another kind of file
// in "Other docs". The helper still says kind "doc".
import { test, beforeEach } from "node:test";
import assert from "node:assert/strict";
import { fake, reset } from "./stub/herdr.mjs";
import { textOf, byId } from "./stub/ui.mjs";
import { fakeCx } from "./fake-cx.mjs";
import { agent, space } from "./fixtures.mjs";
import * as R from "../mods/flockbook/resources.js";
import { forgetCommands } from "../mods/flockbook/sidecar.js";

const doc = (id, target, last_ts = 10) => ({ id, kind: "doc", target, title: target.split("/").pop(), detail: "~/p", row: `r-${id}`, first_ts: last_ts, last_ts, mentions: 1, exists: true });
const link = (id) => ({ id, kind: "link", target: `https://x.com/${id}`, title: id, detail: "x.com", row: `r-${id}`, first_ts: 1, last_ts: 1, mentions: 1 });

beforeEach(() => {
  reset(); forgetCommands(); R.forgetResources(); R.forgetCache();
  fake.spaces = [space("sp1", { tabs: [{ id: "tb-t1", index: 1, name: "1", active: true, status: "idle", panes: 1, settled: false }] })];
  fake.agents = [agent("t1", "idle", { tab: "tb-t1" })];
});

const open = (items) => {
  R.putData({ terminal: "t1", cursor: "c1", items, at: 1 });
  fake.shared[R.seenKey("t1")] = 0;
  fake.props = { view: "resources", overlay: "o7" };
  fake.shared[R.KEY_OPEN] = "o7"; fake.shared[R.KEY_RUN] = R.RUN;
  fake.shared[R.KEY_WIN_W] = 1400; fake.shared["flockbook.window.height"] = 30000;
  return new R.ResourcesView(fakeCx());
};

test("a doc's group comes from its extension, any case; another kind of file is in Other docs", () => {
  const g = (t) => R.groupOf(doc("x", t));
  assert.equal(g("/p/plan.md"), "markdown");
  assert.equal(g("/p/PLAN.MD"), "markdown");
  assert.equal(g("/p/a.html"), "web");
  assert.equal(g("/p/a.HTM"), "web");
  assert.equal(g("/p/a.pdf"), "pdfs");
  assert.equal(g("/p/a.Png"), "images");
  assert.equal(g("/p/a.svg"), "images");
  assert.equal(g("/p/notes.txt"), "otherdocs");
  assert.equal(g("/p/Makefile"), "otherdocs", "no extension");
  assert.equal(g("/p.d/README"), "otherdocs", "a dot in a folder is not an extension");
  assert.equal(R.groupOf(link("l")), "links");
  assert.equal(R.groupOf({ kind: "pr" }), "github");
});

test("the window: Markdown, Web pages, PDFs, Images, Other docs in that order before Artifacts; empty groups not shown", () => {
  const v = open([doc("m", "/p/a.md", 5), doc("h", "/p/b.html", 6), doc("i", "/p/c.png", 7), doc("o", "/p/d.txt", 8), link("l")]);
  const el = v.render(fakeCx());
  const ids = (byId(el, "res-list").kids ?? []).map((k) => k._a.id);
  assert.deepEqual(ids, ["res-group-markdown", "res-group-web", "res-group-images", "res-group-otherdocs", "res-group-links"], "no PDFs group: none");
  assert.match(textOf(byId(el, "res-group-head-markdown")), /Markdown\s*1/);
  assert.match(textOf(byId(el, "res-group-head-web")), /Web pages\s*1/);
  assert.match(textOf(byId(el, "res-group-head-images")), /Images\s*1/);
  assert.match(textOf(byId(el, "res-group-head-otherdocs")), /Other docs\s*1/);
  assert.equal(byId(el, "res-group-docs"), null, "no Docs group any more");
  const pdf = open([doc("p", "/p/x.PDF")]).render(fakeCx());
  assert.match(textOf(byId(pdf, "res-group-head-pdfs")), /PDFs\s*1/);
});

test("each doc group has its own icon on its rows", () => {
  const el = open([doc("m", "/p/a.md"), doc("h", "/p/b.htm"), doc("p", "/p/c.pdf"), doc("i", "/p/d.svg"), doc("o", "/p/e.txt")]).render(fakeCx());
  const iconOf = (id) => { let p = null; const walk = (e) => { if (!e || typeof e !== "object" || p) return; if (e.tag === "svg" && /icons\//.test(e._a.path ?? "") && !/chevron/.test(e._a.path)) { p = e._a.path; return; } (e.kids ?? []).forEach(walk); }; walk(byId(el, `res-${id}`)); return p?.split("/").pop(); };
  assert.deepEqual(["m", "h", "p", "i", "o"].map(iconOf), ["doc.svg", "globe.svg", "file-text.svg", "image.svg", "doc.svg"]);
});

test("each group folds on its own; an old 'docs' in the folded list folds nothing", () => {
  fake.shared[R.KEY_COLLAPSED] = ["docs"];
  const v = open([doc("m", "/p/a.md"), doc("h", "/p/b.html")]);
  let el = v.render(fakeCx());
  assert.ok(byId(el, "res-m") && byId(el, "res-h"), "nothing folded");
  byId(el, "res-group-head-web")._h.on_click(null, fakeCx());
  assert.deepEqual(fake.shared[R.KEY_COLLAPSED], ["docs", "web"]);
  el = v.render(fakeCx());
  assert.ok(byId(el, "res-m"), "Markdown still open");
  assert.equal(byId(el, "res-h"), null, "Web pages folded");
});

test("more doc groups do not change the height (one height)", () => {
  open([doc("m", "/p/a.md")]);
  const one = R.placeNow().height;
  open([doc("m", "/p/a.md"), doc("h", "/p/b.html"), doc("p", "/p/c.pdf")]);
  assert.equal(R.placeNow().height, one);
});
