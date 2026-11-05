// Row heights must never be guessed under what the chat draws: an under-guess cuts the row's last line.
// The drawn sizes below are measured from the app (2× screenshot of a 748 px reading column): prose
// 16 px with 26 px lines, a 10 px gap between paragraphs, 4 px between list items, list text 22 px in,
// a full-width character 16 px wide, inline code Menlo 8.4 px a character plus 4 px padding each side.
import { test } from "node:test";
import assert from "node:assert/strict";
import { markdownHeight, rowHeight } from "../mods/flockbook/estimate.js";

const W = 748;
const LINE = 26, GAP = 10, ITEM_GAP = 4;
// Lines a full-width text takes when drawn: no line starts with full-width punctuation, so a line can
// end one character early.
const drawnCjkLines = (chars, width) => Math.ceil(chars * 16 / (width - 16));

// Synthetic text, shaped like the cut reply: CJK with full-width punctuation, inline code, Latin words, a list.
const reply = [
  "目前還有兩個問題",
  "",
  "- 使用者在地圖外面放開手指時，程式可能收不到結束事件。要用 `on_pointer_cancel_out` 這類事件補上，讓拖動一定會停下。",
  "- 每張卡片寬度都不同，要先知道它們的實際位置，分隔線才放得準。",
  "",
  "範圍： 我建議先做上方的 card 列。左邊的 folder，系統自己已經能拖動排序（說明裡有寫）。",
  "",
  "要不要做？要的話，我等 build-bot 修完 Shift+Enter，再接著交給它。它熟 `grid.js`，也最能避開和設定畫面的衝突，順便補上測試。",
].join("\n");

test("a CJK reply with inline code and a list is not guessed shorter than drawn (the cut last line)", () => {
  // Drawn: 1 + (2 + 1) + 1 + 2 lines; gaps: title to list, between items, list to paragraph, paragraph to paragraph.
  const drawn = 7 * LINE + GAP + ITEM_GAP + GAP + GAP;
  assert.ok(markdownHeight(reply, W) >= drawn, `${markdownHeight(reply, W)} < ${drawn}`);
});

test("short CJK paragraphs of two or three lines get at least their drawn height", () => {
  for (let chars = 40; chars <= 140; chars++) {
    const text = "測".repeat(chars - 1) + "。";
    const drawn = drawnCjkLines(chars, W) * LINE;
    assert.ok(markdownHeight(text, W) >= drawn, `${chars} chars: ${markdownHeight(text, W)} < ${drawn}`);
  }
});

test("a CJK list item is guessed at least as drawn, its indent taken off the width", () => {
  for (let chars = 30; chars <= 120; chars += 3) {
    const text = "- " + "清".repeat(chars);
    const drawn = drawnCjkLines(chars, W - 22) * LINE;
    assert.ok(markdownHeight(text, W) >= drawn, `${chars} chars: ${markdownHeight(text, W)} < ${drawn}`);
  }
});

test("a line full of inline code is guessed at least as drawn", () => {
  const spans = Array.from({ length: 12 }, (_v, i) => `\`on_event_name_${i}\``).join(" ");
  // Each span: 15 or 16 characters at 8.4 px + 8 px padding, then a 4.5 px space.
  const width = Array.from({ length: 12 }, (_v, i) => `on_event_name_${i}`.length * 8.4 + 8 + 4.5).reduce((a, b) => a + b, 0);
  const drawn = Math.ceil(width / (W - 40)) * LINE;  // a span does not break: a line can end a whole span early
  assert.ok(markdownHeight(spans, W) >= drawn, `${markdownHeight(spans, W)} < ${drawn}`);
});

test("a long English answer is not guessed far taller than drawn (no big gap)", () => {
  // About 0.47 em a letter, spaces included: 7.5 px at 16 px.
  const text = Array.from({ length: 6 }, () => "The helper reads the log on that machine and sends the lines back as they are. ".repeat(4)).join("\n\n");
  const perPara = Math.ceil(("The helper reads the log on that machine and sends the lines back as they are. ".repeat(4)).length * 7.5 / W);
  const drawn = 6 * perPara * LINE + 5 * GAP;
  const guess = markdownHeight(text, W);
  assert.ok(guess >= drawn, `${guess} < ${drawn}`);
  assert.ok(guess <= drawn * 1.3, `${guess} is more than 30% over ${drawn}`);
});

test("an assistant row is its text's guess plus 16", () => {
  assert.equal(rowHeight({ k: "assistant", text: reply }, W), markdownHeight(reply, W) + 16);
});
