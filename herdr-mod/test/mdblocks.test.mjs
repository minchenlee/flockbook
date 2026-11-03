import { test } from "node:test";
import assert from "node:assert/strict";
import { splitBlocks, tableCells } from "../mods/flockbook/mdblocks.js";
import { markdownView } from "../mods/flockbook/mdblocks.js";
import { textOf } from "./stub/ui.mjs";
import { cellWidth, markdownHeight } from "../mods/flockbook/estimate.js";

test("code fences and tables come out as their own blocks; the rest stays markdown", () => {
  const text = "Intro **bold**\n\n```js\nconst a = 1;\n  b();\n```\nMiddle\n\n| Name | Size |\n|:--|--:|\n| `a` | 1 |\n| b | 22 |\n\nEnd";
  const b = splitBlocks(text);
  assert.deepEqual(b.map((x) => x.k), ["md", "code", "md", "table", "md"]);
  assert.equal(b[1].lang, "js");
  assert.equal(b[1].text, "const a = 1;\n  b();");
  assert.deepEqual(b[3].head, ["Name", "Size"]);
  assert.deepEqual(b[3].align, ["left", "right"]);
  assert.deepEqual(b[3].rows, [["`a`", "1"], ["b", "22"]]);
  assert.equal(b[4].text.trim(), "End");
});

test("an unclosed fence runs to the end; a pipe line without a separator is text", () => {
  assert.deepEqual(splitBlocks("x\n```\nopen").map((x) => x.k), ["md", "code"]);
  assert.deepEqual(splitBlocks("a | b\n| not a table").map((x) => x.k), ["md"]);
  assert.deepEqual(splitBlocks("plain").map((x) => x.k), ["md"]);
});

test("cells split on pipes, not on escaped ones or pipes inside code", () => {
  assert.deepEqual(tableCells("| a \\| b | `x|y` | c |"), ["a | b", "`x|y`", "c"]);
});

test("the view: rounded code box keeps the indent; rounded table with a header", () => {
  const T = new Proxy({}, { get: (_t, k) => `#${k}` });
  const el = markdownView("r1", "```\n  indented\n```\n| H |\n|---|\n| v |", T);
  const all = []; (function walk(n) { if (!n) return; all.push(n); (n.kids ?? []).forEach(walk); })(el);
  const boxes = all.filter((n) => n._a?.rounded);
  assert.ok(boxes.length >= 2, "both blocks are rounded");
  assert.match(textOf(el), /  indented/);
  assert.match(textOf(el), /H.*v/);
});

test("table cells have a fixed width, so long text wraps inside the table; estimate grows with it", () => {
  const T = new Proxy({}, { get: (_t, k) => `#${k}` });
  const long = "a very long cell ".repeat(20);
  const text = `x\n| A | B |\n|---|--:|\n| ${long} | 1 |`;
  const el = markdownView("r2", text, T, 602);
  const all = []; (function walk(n) { if (!n) return; all.push(n); (n.kids ?? []).forEach(walk); })(el);
  const cells = all.filter((n) => n._a && n._a.px === 12 && n._a.py === 8);
  assert.equal(cells.length, 4);
  for (const c of cells) assert.equal(c._a.w, cellWidth(602, 2), "fixed width, not a flexible cell");
  assert.equal(cellWidth(602, 2), 300);
  assert.ok(cells.some((c) => "text_right" in c._a), "right-aligned column");
  const short = markdownHeight("x\n| A | B |\n|---|--:|\n| a | 1 |", 602);
  assert.ok(markdownHeight(text, 602) >= short + 4 * 21, "a wrapped row is guessed taller");
});
