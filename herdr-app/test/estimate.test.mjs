// Row heights must never be guessed under what the chat draws: an under-guess cuts the row's last line.
// The drawn sizes below are measured from the app (2× screenshot of a 748 px reading column): prose
// 16 px with 26 px lines, a 10 px gap between paragraphs, 4 px between list items, list text 22 px in,
// a full-width character 16 px wide, inline code Menlo 8.4 px a character plus 4 px padding each side.
import { test } from "node:test";
import assert from "node:assert/strict";
import { markdownHeight, rowHeight, lineHeight } from "../mods/flockbook/estimate.js";

const W = 748;
const LINE = 26, GAP = 10, ITEM_GAP = 4;
// Lines a full-width text takes when drawn: gpui fills each line to its width and lets punctuation start
// a line (measured in the VM, Linux herdr-app 0.1.13: 35 characters fit a 564 px bubble, 36 wrap; gpui's
// line wrapping is the same code on macOS).
const drawnCjkLines = (chars, width) => Math.ceil(chars / Math.floor(width / 16));

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

test("line heights round like gpui's: whole px of the window, so a zoom or a text size can round them up", () => {
  assert.deepEqual([lineHeight(16), lineHeight(13), lineHeight(13.5)], [26, 21, 22], "the measured heights at zoom 1");
  // Zoom 0.9: code lines are drawn 19 px apart (measured), 21.11 px at zoom 1; counted as 21, a 113-line
  // code box came out 12 px short and cut the reply's last line.
  assert.equal(lineHeight(13, 0.9), 19 / 0.9);
  const code = "```\n" + Array.from({ length: 113 }, (_v, i) => `line_${i} = ${i}`).join("\n") + "\n```";
  assert.ok(markdownHeight(code, W, null, 1, 0.9) >= 34 + 113 * 19 / 0.9);
  // Small text (14 px): prose lines are 22.65 px, drawn 23 on a 1× screen and 22.5 on a 2× one; 23 covers both.
  assert.equal(lineHeight(14), 23);
  // 21.3 px is drawn 21.5 on a 2× screen: counted as 21 it would come out short.
  assert.equal(lineHeight(21.3 / 1.618), 21.5);
  const para = "目".repeat(400);
  assert.ok(markdownHeight(para, W, null, 14 / 16) >= Math.ceil(400 / Math.floor(W / 14)) * 23);
});

test("text size: the default guesses as before; a smaller size is shorter but not under its own drawing", () => {
  assert.equal(markdownHeight(reply, W, null, 1), markdownHeight(reply, W));
  const small = markdownHeight(reply, W, null, 14 / 16), large = markdownHeight(reply, W, null, 18 / 16);
  assert.ok(small < markdownHeight(reply, W) && markdownHeight(reply, W) < large);
  // Full-width text at 14 px: 14 px characters on 23 px lines.
  const para = "目".repeat(120);
  assert.ok(markdownHeight(para, W, null, 14 / 16) >= Math.ceil(120 / Math.floor(W / 14)) * lineHeight(14));
  assert.equal(rowHeight({ k: "assistant", text: reply }, W, false, 1), rowHeight({ k: "assistant", text: reply }, W));
});

test("lines ending in two spaces or a backslash are hard breaks: each starts a line, as drawn", () => {
  // A report shaped like Codex's: label lines ended with two spaces, then a paragraph after a code box.
  const lines = ["Result: **the call worked.**", "To do: none", "Open: why the earlier call failed", "Checked: one row came back", "Branch: none"];
  const twoSpaces = lines.join("  \n");
  assert.ok(markdownHeight(twoSpaces, W) >= lines.length * LINE, `${markdownHeight(twoSpaces, W)} < ${lines.length * LINE}`);
  const backslash = lines.join("\\\n");
  assert.ok(markdownHeight(backslash, W) >= lines.length * LINE, `${markdownHeight(backslash, W)} < ${lines.length * LINE}`);
  // A soft break still joins: the same lines without the marks are one wrapped paragraph.
  assert.ok(markdownHeight(lines.join("\n"), W) < lines.length * LINE);
});

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

// A user bubble's text width at a 760 px column: 78% of the column, less 14 px padding each side.
const BUBBLE = Math.floor(760 * 0.78) - 28;

test("a paragraph pasted with short source lines is guessed as one joined paragraph", () => {
  // Markdown joins lines that start no new block (a soft break draws as a space): this draws as one line
  // (measured in the VM). Counted line by line it was guessed three lines tall.
  const text = "Line one of a paragraph\nline two joins it\nline three joins it too";
  const guess = markdownHeight(text, BUBBLE);
  assert.ok(guess >= LINE && guess < 2 * LINE, `${guess}`);
});

test("a long pasted message with a list is guessed within one line of its drawn height", () => {
  // Synthetic, shaped like the long pasted worker message that left a big gap under its bubble: a title, a
  // numbered list with nested items, CJK mixed with commands and paths, a line that continues an item.
  const text = [
    "worker-a：relay 計畫（等你同意才寫程式）",
    "1. Server：新增 `serve` 子命令，預設 port 8787，只聽 127.0.0.1；加 --bind 才對外。原因：手機只透過 Tailscale 連進來，不需要對整個網路開 port，這樣也不用處理 TLS 憑證。",
    "2. Login：第一次啟動產生一組 token，存在 state 目錄，權限 0600。",
    "   - on：寫檔、載入（systemctl --user enable --now），然後等 health 回 200 才算成功。",
    "   - off：只在檔案有我們的標記時才停掉並刪那一個檔；手動裝的檔案不動。",
    "   - status：看 systemctl --user is-active，再連一下 127.0.0.1:port。",
    "3. Client：設定頁加一欄 URL 和一欄 token，存檔後馬上重連。",
    "重連失敗時顯示一行錯誤，不跳視窗。",
    "4. Tests：unit 12 個，integration 3 個（含斷線重連、token 錯誤、port 被占用）。",
    "   - Rust：temp HOME，PATH 上放假的 systemctl，記錄被呼叫的引數。",
    "   - npm：設定區塊的各種狀態、鍵盤操作、Copy、--home 有傳。",
    "   - VM：在 test-vm 用真的 systemd --user（我自己的 HOME/session）。",
    "   - Docs：RECIPE、SERVE.md、next.md 各加一段。",
    "問題：1) QR 要不要做（多一個 qrcode crate）？2) Port 和 token 用文字輸入欄可以嗎？還是 Port 先固定 8787，只讓改 token？",
  ].join("\n");
  // Drawn text height, measured in the VM (Linux herdr-app 0.1.13, 760 px column): the bubble is 554 px,
  // 20 px of it padding. The guess was 1.18 times that before (8% on top, a character of slack a line,
  // each source line counted alone).
  const drawn = 534;
  const guess = markdownHeight(text, BUBBLE);
  assert.ok(guess >= drawn, `${guess} < ${drawn}`);
  assert.ok(guess - drawn < LINE, `${guess} is a line or more over ${drawn}`);
});

// Latin words drawn on macOS (2× screenshot of a 760 px column, Herdr app 0.1.13): "Ran a command" is
// about 112 px, "Herdr" with its two spaces about 44 px; a full-width character 16 px.
test("a list item whose Latin words push the last full-width mark past the edge is guessed two lines", () => {
  // Drawn: 「 + "Ran a command" + 」 + 36 full-width characters = 16 + 112 + 16 + 576 = 720 px, then 。
  // (16 px) does not fit the item's 737 px (760 less the 22 px indent) and goes to a line of its own.
  const text = "1. 「Ran a command」" + "測".repeat(36) + "。";
  const guess = markdownHeight(text, 760);
  assert.ok(guess >= 2 * LINE, `${guess} < ${2 * LINE}`);
});

test("digits and an en dash in a CJK list item are not counted narrower than drawn", () => {
  // " 17–23" (space, digits, en dash) is drawn about 48 px wide; counted at 0.5 em it was 44.8 px. Drawn:
  // 32 full-width characters + 5 × 48 = 752 px, wider than the item's 737 px: two lines.
  const text = "- " + "測".repeat(32) + " 17–23".repeat(5);
  assert.ok(markdownHeight(text, 760) >= 2 * LINE, `${markdownHeight(text, 760)}`);
});

test("bold markers are not counted as drawn text", () => {
  // "**" is markup: a line of 46 full-width characters (736 px) in bold fits a 748 px column.
  const text = "**" + "粗".repeat(46) + "**";
  assert.ok(markdownHeight(text, W) < 2 * LINE, `${markdownHeight(text, W)}`);
});

test("a user row with pictures counts the \"N images\" line under its text (4 px gap, an 11.5 px line)", () => {
  for (const text of ["hi", "", "目前還有兩個問題，使用者在地圖外面放開手指時。".repeat(4)]) {
    const plain = rowHeight({ k: "user", text }, W), with2 = rowHeight({ k: "user", text, images: 2 }, W);
    // gpui's line height is about 1.618 × the size: 18.6 px, counted 19.
    assert.equal(with2 - plain, 4 + 19, JSON.stringify(text));
  }
});

// A table whose first column is narrow and starts with long inline code (2× screenshot of a 760 px column,
// Herdr app, 11-10): cell text 13.5 px on 22 px lines; inline code Menlo 11.8 px (7.05 px a character) with
// about 4.5 px padding each side; a code word that does not fit breaks after its hyphen; a full-width ）
// that does not fit goes to a line of its own. A row is 8 + 8 px padding and a 1 px line, plus its tallest
// cell; the table 6 px above and below and a 1 px frame. Synthetic text of the same shape as the cut reply.
const narrowFirst = [
  "| 工具 | 現在的問題 | 新版改了什麼 |",
  "|---|---|---|",
  "| `kite-press`（收件的看門員） | 風箏線收不回來時只顯示「失敗」，沒有原因；Kite 的 `.spool`、舊的備用線軸都被拒 | 多三條收線規則：線軸的下載副本、倉庫資料夾裡面的子資料夾、有登記過的備用線軸。`--explain` 只看不收，會清楚列出每一項沒通過的檢查和下一步要做什麼。每次收線都會寫進同一份新的紀錄檔 `kite-moves.jsonl` |",
  "| `kite-guard`（每 30 分鐘量一次剩餘空間） | 風太大的時候要叫 `kite-inventory` 點名，但從 10-29 起一次都沒送出 | 找到原因了：排程的工作一結束，就把背景跑的 `kite-inventory` 一起停掉。改成在前景跑，大約 20 秒。另外每 30 分鐘寫一行時間線，之後才能看出風是在什麼時候、怎麼變大的 |",
  "| `kite-inventory`（點名 admin 清理） | 同上，通知送不出去 | 修好送出；單位改成正確的 m/s |",
  "| `kite-deletes-read`（新增） | — | 讀收線紀錄：誰、什麼時候、收了什麼、多長 |",
].join("\n");

test("a narrow table column with long inline code is guessed at least as drawn (the cut reply)", () => {
  // First column, 78 px inside at 760: kite- / press（收 / 件的看門員 / ） = 4 lines; kite- / guard（每 /
  // 30 分鐘量一 / 次剩餘空間 / ） = 5; kite- / inventory / （點名 / admin 清理 / ） = 5; kite- / deletes- /
  // read（新增 / ） = 4. The other columns take fewer lines.
  const drawn = 6 + 6 + 2 + (16 + 22) + [4, 5, 5, 4].reduce((a, n) => a + 17 + 22 * n, 0);
  const guess = markdownHeight(narrowFirst, 760);
  assert.ok(guess >= drawn, `${guess} < ${drawn}`);
  assert.ok(guess - drawn < 22, `${guess} is a line or more over ${drawn}`);
});
