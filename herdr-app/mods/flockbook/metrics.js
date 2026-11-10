// The chat's row sizes, in px: one set for the drawing (chat.js), the row height guess (estimate.js) and
// the right-click menu's offsets (chat.js codeMenu). Change a size here and all three follow.
// Pure (no gpui): the node tests import it as it is.

// Above and below a user or assistant row's content.
export const ROW_PAD_Y = 8;

// The user's bubble: at most BUBBLE_W of the column, padded inside.
export const BUBBLE_W = 0.78;
export const BUBBLE_PAD_X = 14;
export const BUBBLE_PAD_Y = 10;
// The bubble's outer width and its text's width, for a column `width` px wide.
export const bubbleWidth = (width) => Math.floor(width * BUBBLE_W);
export const bubbleTextWidth = (width) => bubbleWidth(width) - 2 * BUBBLE_PAD_X;

// A row of chips (tool calls, a note from another agent): the chip and the row's padding above and below.
export const CHIP_H = 28;
export const CHIP_PAD_Y = 6;
export const CHIP_ROW = CHIP_H + 2 * CHIP_PAD_Y;   // 40
// Open: what shows under the chips, CHIP_GAP below them.
export const CHIP_GAP = 4;
// What an open chip shows (a note's text, the calls): this far in from the chips.
export const OPEN_INDENT = 12;
// An open tool chip: one line a call, and 8 px more under the calls (the gap above them and 4 px spare).
export const CALL_H = 26;
export const CALLS_PAD = 8;

// Rows of one fixed height.
export const SEP_H = 40;
export const FOOTER_H = 30;
export const STATUS_H = 34;
export const CARD_ROW_H = 44;   // a running call (perm) or a question
export const OTHER_H = 30;
