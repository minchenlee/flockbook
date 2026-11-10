// Golden row heights: the numbers the chat guesses today for a fixed set of texts and rows. A refactor of
// estimate.js, mdblocks.js or chat.js must keep them exactly; a change in behaviour updates them on purpose,
// in its own commit, with a test of its own.
// Also: the estimate (markdownHeight's code box spans) and the drawing (splitBlocks) must see the same code
// boxes. Both use blocks.js; a text in KNOWN_MISMATCH would be one they still split differently (none now).
import { test } from "node:test";
import assert from "node:assert/strict";
import { markdownHeight, rowHeight } from "../mods/flockbook/estimate.js";
import { splitBlocks } from "../mods/flockbook/mdblocks.js";
import { TEXTS, KNOWN_MISMATCH, WIDTHS, ROWS } from "./rowgolden.texts.mjs";

// markdownHeight at WIDTHS (748, 400).
const MARKDOWN = {
  short: [32, 32], latin: [110, 188], latinParas: [104, 182], softBreaks: [32, 58], cjk: [84, 136], cjkReply: [160, 238],
  inlineCode: [84, 162], nestedList: [178, 178], listAfterPara: [128, 128], heading: [166, 166], quote: [58, 58],
  rule: [104, 104], table: [128, 149], tableInText: [158, 158], code: [134, 134], codeLong: [76, 118],
  twoCodes: [163, 163], codeUnclosed: [108, 108],
  // Were 68: the estimate saw no code box in a ~~~ fence and cut the drawn box off. fenceInfo kept its number
  // (the estimate counted its box already; the drawing now sees it too).
  tildeFence: [129, 129], fenceInfo: [87, 87],
};

// rowHeight at WIDTHS.
const ROW = {
  // The image rows were 68 and 36: the "N images" line under the text was not counted (fix/user-image-row).
  "user": [222, 326], "user short": [68, 68], "user images": [91, 91], "user only images": [59, 59],
  "note": [40, 40], "note open": [148, 226], "assistant": [150, 150], "assistant table": [144, 165],
  "tools": [40, 40], "tools open": [126, 126], "sep": [40, 40], "footer": [30, 30], "status": [34, 34],
  "perm": [44, 44], "question": [44, 44], "other": [30, 30],
};

test("golden: every text has its numbers, and every number its text", () => {
  assert.deepEqual(Object.keys(MARKDOWN).sort(), Object.keys(TEXTS).sort());
  assert.deepEqual(Object.keys(ROW).sort(), ROWS.map(([n]) => n).sort());
});

for (const [name, text] of Object.entries(TEXTS)) {
  test(`golden markdownHeight: ${name}`, () => {
    assert.deepEqual(WIDTHS.map((w) => markdownHeight(text, w)), MARKDOWN[name]);
  });
}

for (const [name, row, open] of ROWS) {
  test(`golden rowHeight: ${name}${open ? " (open)" : ""}`, () => {
    assert.deepEqual(WIDTHS.map((w) => rowHeight(row, w, open)), ROW[name]);
  });
}

// The code boxes each side sees: the drawing's code blocks and the estimate's spans, as their source lines.
function boxes(text) {
  const spans = [];
  markdownHeight(text, WIDTHS[0], spans);
  return {
    drawn: splitBlocks(text).filter((b) => b.k === "code").map((b) => b.text.split("\n").map((l) => l.replace(/\r$/, "")).join("\n")),
    estimated: spans.map((s) => s.lines.join("\n")),
  };
}

for (const name of Object.keys(TEXTS)) {
  const known = KNOWN_MISMATCH.has(name);
  test(`the estimate and the drawing see the same code boxes: ${name}`, known ? { todo: "known mismatch: one block parser fixes it" } : {}, () => {
    const { drawn, estimated } = boxes(TEXTS[name]);
    assert.deepEqual(estimated, drawn);
  });
}
