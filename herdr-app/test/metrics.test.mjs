// The derived row sizes keep the numbers the drawing and the estimate used before they were shared.
import { test } from "node:test";
import assert from "node:assert/strict";
import { CHIP_ROW, bubbleWidth, bubbleTextWidth, ROW_PAD_Y, BUBBLE_PAD_Y } from "../mods/flockbook/metrics.js";

test("a chip row is 40 px; the bubble 78% of the column, its text 28 px narrower", () => {
  assert.equal(CHIP_ROW, 40);
  assert.equal(bubbleWidth(760), 592);
  assert.equal(bubbleTextWidth(760), 564);
});

test("the text in a user bubble starts 18 px down its row, in an assistant row 8 px (the right-click offsets)", () => {
  assert.equal(ROW_PAD_Y + BUBBLE_PAD_Y, 18);
  assert.equal(ROW_PAD_Y, 8);
});
