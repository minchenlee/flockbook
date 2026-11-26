import { test } from "node:test";
import assert from "node:assert/strict";
import { splitBlocks, tableCells } from "../mods/flockbook/mdblocks.js";
import { markdownView } from "../mods/flockbook/mdblocks.js";
import { textOf } from "./stub/ui.mjs";
import { colWidths, markdownHeight } from "../mods/flockbook/estimate.js";

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

test("text size: the default sets no size; another sets it on the text, code boxes and tables, scaled", () => {
  const T = new Proxy({}, { get: (_t, k) => `#${k}` });
  const all = (el) => { const out = []; (function walk(n) { if (!n) return; out.push(n); (n.kids ?? []).forEach(walk); })(el); return out; };
  const text = "Intro\n\n```\ncode\n```\n| H |\n|---|\n| v |";
  assert.ok(all(markdownView("r", text, T, 600)).every((n) => n._a?.text_size !== 16), "no size at the default");
  assert.equal(all(markdownView("r", "plain", T, 600, 1))[0]._a?.text_size, undefined, "one block stays a bare view");
  const small = all(markdownView("r", text, T, 600, 14 / 16)).map((n) => n._a?.text_size).filter(Boolean);
  assert.deepEqual(small, [14, 13 * 14 / 16, 13.5 * 14 / 16]);
  assert.equal(all(markdownView("r", "plain", T, 600, 18 / 16))[0]._a?.text_size, 18);
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
  const ws = colWidths(602, [["A", "B"], [long.trim(), "1"]], 2);
  assert.deepEqual(cells.slice(0, 2).map((c) => c._a.w), ws, "fixed widths, not flexible cells");
  assert.equal(ws[0] + ws[1], 600);
  assert.ok(ws[1] < 100 && ws[0] > 500, "the short column stays narrow, the long one takes the rest");
  assert.ok(cells.some((c) => "text_right" in c._a), "right-aligned column");
  const short = markdownHeight("x\n| A | B |\n|---|--:|\n| a | 1 |", 602);
  // The long cell is drawn on 4 lines of 22 px (word wrap at its width), the short one on 1.
  assert.ok(markdownHeight(text, 602) >= short + 3 * 22, "a wrapped row is guessed taller");
});

test("column widths follow the content: short columns keep their size, the space left goes by length", () => {
  const rows = [["時間", "階段", "我做的事", "d2r 的角色"],
    ["10-22 下午", "規劃", "拆解上游的架構，查授權，寫計畫（M0–M4、驗收、風險）。".repeat(3), "比較的對象"]];
  const ws = colWidths(1000, rows, 4);
  assert.equal(ws.reduce((a, b) => a + b, 0), 998);
  assert.ok(ws[0] < 140 && ws[1] < 100, `short columns narrow: ${ws}`);
  assert.ok(ws[2] > ws[3] && ws[2] > 500, `the long column is widest: ${ws}`);
  // All fit: every column grows in proportion, nothing is cut.
  const fit = colWidths(1000, [["a", "bbbbbbbb"]], 2);
  assert.equal(fit[0] + fit[1], 998);
  assert.ok(fit[1] > fit[0]);
  // Narrow table: no column under 64.
  assert.ok(colWidths(200, rows, 4).every((w) => w >= 64));
});

// What a reader sees in the code box's TextView: markdown escapes and hard breaks undone.
const shown = (md) => md.replace(/\\\n/g, "\n").replace(/\\([!-/:-@[-`{-~])/g, "$1");
const nodes = (el) => { const all = []; (function walk(n) { if (!n) return; all.push(n); (n.kids ?? []).forEach(walk); })(el); return all; };

test("a code box is one selectable text, so a drag selects across its lines", () => {
  const T = new Proxy({}, { get: (_t, k) => `#${k}` });
  const el = markdownView("r1", "Run this:\n\n```sh\ntailscale serve --bg --https=8443 8787\n  echo *ok* # [x](y) <b>\n\n\tdone\n```", T);
  const views = nodes(el).filter((n) => n.tag === "md");
  const code = views.find((v) => /tailscale/.test(v._a.text));
  assert.ok(code, "the code is in a TextView");
  assert.equal(code._a.selectable, true);
  assert.equal(shown(code._a.text), "tailscale serve --bg --https=8443 8787\n  echo *ok* # [x](y) <b>\n \n    done",
    "every line in one view; indent kept; markup drawn as text; a blank line kept");
  assert.ok(nodes(el).some((n) => n._a?.rounded === 12 && nodes(n).includes(code)), "inside our rounded box");
});

test("code box text: CRLF lines lose the CR; a line ending in a backslash keeps it", () => {
  const T = new Proxy({}, { get: (_t, k) => `#${k}` });
  const el = markdownView("r2", "```\na\r\nb \\\r\nc\n```", T);
  const code = nodes(el).find((n) => n.tag === "md");
  assert.equal(shown(code._a.text), "a\nb \\\nc");
});

test("Copy text is the block as written: normal spaces and tabs, no no-break spaces", async () => {
  const { codeCopyText } = await import("../mods/flockbook/mdblocks.js");
  const b = splitBlocks("```\n  two\n\tone tab\r\n\nend\n```")[0];
  assert.equal(codeCopyText(b), "  two\n\tone tab\n\nend");
  assert.ok(!codeCopyText(b).includes(" "));
});

test("a code line is guessed as tall as a TextView line at 13 px (21 px), not shorter", () => {
  const one = markdownHeight("```\nx\n```", 760), three = markdownHeight("```\nx\ny\nz\n```", 760);
  assert.equal(three - one, 2 * 21);
});

test("codeBlockAt finds the code box under a point: several boxes, a box at the top and at the bottom", async () => {
  const { codeBlockAt } = await import("../mods/flockbook/mdblocks.js");
  const W = 700;
  const before = "Intro line.\n\n";
  const text = `${before}\`\`\`sh\n  first\n\`\`\`\nMiddle words here.\n\n\`\`\`\nsecond\n\tline\n\`\`\`\nEnd.`;
  const top1 = markdownHeight(before + "x", W) - markdownHeight("x", W) + 6;   // under the prose, past the margin
  assert.equal(codeBlockAt(text, W, top1 + 10), "  first");
  assert.equal(codeBlockAt(text, W, 2), null, "on the prose above");
  const total = markdownHeight(text, W);
  // The second box ends above "End." (one line + the end pad).
  const endH = markdownHeight("End.", W);
  assert.equal(codeBlockAt(text, W, total - endH - 10), "second\n\tline");
  assert.equal(codeBlockAt(text, W, total - 5), null, "on the last line");
  // Between the boxes: the middle paragraph.
  assert.equal(codeBlockAt(text, W, top1 + 6 + 19 + 21 + 8), null);
  // A text that is only a box: top edge and bottom edge.
  const only = "```\na\r\nb\n```";
  assert.equal(codeBlockAt(only, W, 7), "a\nb");
  assert.equal(codeBlockAt(only, W, markdownHeight(only, W) - 7), "a\nb");
  assert.equal(codeBlockAt(only, W, 3), null, "in the margin");
  assert.equal(codeBlockAt("no code at all", W, 5), null);
});

test("text view ids stay the same when the message grows a code box (a redraw keeps the selection)", () => {
  const T = new Proxy({}, { get: (_t, k) => `#${k}` });
  const ids = (text) => { const out = []; (function walk(n) { if (!n) return; if (n._a?.id) out.push(n._a.id); (n.kids ?? []).forEach(walk); })(markdownView("r1", text, T, 600)); return out; };
  assert.deepEqual(ids("First paragraph."), ["md-r1-0"], "one block: the same id as the first of many");
  const grown = ids("First paragraph.\n\n```\nx\n```\nMore.");
  assert.equal(grown[0], "md-r1-0", "the first text keeps its id when a code box comes after it");
  assert.equal(new Set(grown).size, grown.length, "ids are unique in the message");
});
