// Row heights for the virtual list, guessed from the text (the list needs them before layout).
// Body text as TextView draws it (measured in the app): 16 px (1rem), 26 px per line (gpui's line
// height, about 1.618 × the size). Over-guessing leaves a gap; under-guessing cuts the row, so the
// guesses lean high: prose wraps by unit (a Latin word or an inline code span does not break, closing
// full-width punctuation stays with the character before it, opening punctuation with the one after),
// letters are counted per character a little wide, and END_PAD px go on the end.
// No percentage on top: on a long pasted message 8% of the height was many lines of empty space.
import { splitBlocks, tableCells } from "./blocks.js";

export const READ_W = 760;  // the reading column's width at most
export const GUTTER = 24;   // each side of the column

const isWide = (c) => c >= 0x1100 && (c <= 0x115f || (c >= 0x2e80 && c <= 0xa4cf) || (c >= 0xac00 && c <= 0xd7a3) ||
  (c >= 0xf900 && c <= 0xfaff) || (c >= 0xfe30 && c <= 0xfe4f) || (c >= 0xff00 && c <= 0xff60) || (c >= 0xffe0 && c <= 0xffe6));

// Body text size and line height, in px.
export const FS = 16, LH = 26;
// Kept under the prose, for what the count misses (a bold word, an emoji). Small: the guess is meant to
// be within a line of the drawn height.
const END_PAD = 6;
// Latin letters in prose: Helvetica's widths (1/1000 em, ASCII 33-126), 6% wider. The Mac's system font draws
// a hyphenated lowercase word 6% wider than Helvetica and digits 0.6 em (measured, Herdr app 0.1.13); a flat 0.5 em was
// narrower than drawn on digits and capitals, and a line that just fit in the guess wrapped on screen.
const HELV = [278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278,
  556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 278, 278, 584, 584, 584, 556, 1015,
  667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611,
  278, 278, 278, 469, 556, 333,
  556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500, 222, 833, 556, 556, 556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500,
  334, 260, 334, 584];
const LATIN_SCALE = 1.06;  // more puts the long VM-measured message (Linux font) a line over
// Other narrow characters (an en dash, an accented letter): 0.6 em.
const latinWidth = (u, fs) => {
  let w = 0;
  for (const ch of u) { const c = ch.codePointAt(0); w += c >= 33 && c <= 126 ? HELV[c - 33] : 600; }
  return w * LATIN_SCALE * fs / 1000;
};
// Inline code: Menlo at 14 px, 0.6 em a character, 4 px padding each side.
const CODE_CH = 14 * 0.6, CODE_PAD = 8;
// Full-width punctuation that may not start a line: it goes down with the character before it.
const NO_START = new Set("，。、；：？！）」』】〉》〕｝％…—～·");
// Full-width punctuation that may not end a line: it goes down with the character after it.
const NO_END = new Set("（「『【〈《〔｛");

// Lines of prose drawn `width` px wide at `fs` px. gpui fills a line to its full width and lets
// punctuation start a line (measured in the VM: 35 full-width characters fit 564 px, 36 wrap), so no
// slack is kept: a character of slack per line counted a line too many on 5 of 54 measured paragraphs
// and none too few without it. Latin letters, counted wide, keep their own margin.
export function proseLines(text, width, fs = FS) {
  width = Math.max(fs, width);
  // A word: letters, digits, brackets and the marks gpui keeps inside one. A slash starts a new word:
  // gpui breaks "HC_SESSION/HC_SOCKET" before the slash (measured in the VM).
  const units = String(text).match(/`[^`]+`|\/?[A-Za-z0-9_\-+.:@#%&=?!'"$<>~^*()[\]{}]+|\s+|./gsu) ?? [];
  const wide = (u) => isWide(u.codePointAt(0));
  const size = (u) => u[0] === "`" && u.length > 1 ? (u.length - 2) * CODE_CH + CODE_PAD
    : /^\s+$/.test(u) ? fs * 0.3 * u.length
      : wide(u) ? fs : latinWidth(u, fs);
  let n = 1, w = 0;
  for (let i = 0; i < units.length; i++) {
    let u = units[i], uw = size(u);
    // Keep opening punctuation with the unit after it, and a run of no-start punctuation with the unit before it.
    while (NO_END.has(u) && i + 1 < units.length && !/^\s+$/.test(units[i + 1])) { u = units[++i]; uw += size(u); }
    while (i + 1 < units.length && NO_START.has(units[i + 1])) { uw += fs; i++; }
    if (w > 0 && w + uw > width) {
      if (/^\s+$/.test(u)) { n++; w = 0; continue; }  // a space at the break is not drawn
      n++; w = 0;
    }
    // Wider than the line (a long word or span): it breaks where it must.
    while (uw > width) { n++; uw -= width; }
    w += uw;
  }
  return n;
}

export function wrapLines(text, width, fs = 14) {
  let w = 0, n = 1;
  for (let i = 0; i < text.length; i++) {
    const cw = isWide(text.charCodeAt(i)) ? fs : fs * 0.58;
    if (w + cw > width) { n++; w = cw; } else w += cw;
  }
  return n;
}

// `spans`, when given, gets each code box as drawn: { top, bottom, lines } in px from the text's top
// (the box itself, without its 6 px margin) and its source lines, for finding the box under a point.
export function markdownHeight(text, width, spans = null) {
  const CODE_LH = 21;  // TextView at 13 px (Menlo): its own line height, as table text (13.5 px) draws 21
  let h = 0, p = 0, prevBlank = true;  // h: drawn exactly; p: prose, wraps
  // A blank line is a 10 px paragraph gap, but not next to a code box or table: their margin is the gap.
  let gap = 0;
  const box = () => { h -= gap; gap = 0; };
  // The open paragraph, list item or quote. Markdown joins a line that starts no new block to it (a soft
  // break draws as a space), so its lines are counted once, joined: counted one by one, a long pasted
  // message with short source lines came out many lines too tall.
  let para = null, lastLevel = null;  // lastLevel: the last list item's nesting, while in a list
  // "**" (bold) is markup, not drawn.
  const close = () => { if (para) p += proseLines(para.text.replace(/\*\*/g, ""), para.width, para.fs) * para.lh + para.extra; para = null; };
  // The blocks mdblocks.js draws (blocks.js): code boxes and tables on their own, the rest markdown.
  for (const b of splitBlocks(text)) {
    if (b.k === "code") {
      // A code box as mdblocks.js draws it: 6 px margin, 10 px padding and a 1 px border, each side.
      close(); lastLevel = null; box();
      if (spans) spans.push({ top: h + p + 6, bottom: 0, lines: [] });
      h += 34;
      for (const raw of b.lines) {
        if (spans) spans.at(-1).lines.push(raw.replace(/\r$/, ""));
        h += CODE_LH * Math.max(1, wrapLines(raw.replace(/\t/g, "    "), width - 30, 13));
      }
      if (spans) spans.at(-1).bottom = h + p - 6;
      prevBlank = true;
      continue;
    }
    if (b.k === "table") {
      // A table as mdblocks.js draws it: 6 px above and below, a 1 px frame; each row 8 px padding top
      // and bottom plus a 1 px line, as tall as its tallest cell at the colWidths widths.
      close(); lastLevel = null;
      const rows = [b.head, ...b.rows];
      const n = Math.max(...rows.map((r) => r.length));
      const ws = colWidths(width, rows, n);
      // The paragraph gap above a table stays (unlike a code box's): as counted before the shared parser.
      gap = 0; h += 14;
      for (const r of rows) h += 17 + 21 * Math.max(1, ...ws.map((w, c) => wrapLines(r[c] ?? "", w - 24, 13.5)));
      prevBlank = true;
      continue;
    }
    for (const raw of b.text.split("\n")) {
      const line = raw.replace(/\t/g, "    ");
      if (!line.trim()) { close(); if (!prevBlank) { h += 10; gap = 10; } prevBlank = true; continue; }
      prevBlank = false; gap = 0;
      let m;
      if ((m = /^(#{1,6})\s/.exec(line))) { close(); lastLevel = null; const size = [30, 25, 22, 20, 17, 16][m[1].length - 1]; p += proseLines(line, width, size) * size * 1.62 + 12; continue; }
      if ((m = /^(\s*)([-*+]|\d+\.)\s/.exec(line))) {
        // Text is 22 px in per level (measured). A list right under a paragraph (no blank line) still gets
        // the paragraph gap above it.
        // Items are 4 px apart; the first item of a list, nested ones too, has no gap above it (measured).
        const level = m[1].length ? Math.ceil(m[1].length / 4) : 0;
        if (para && !para.item) p += 10;
        else if (lastLevel !== null && level <= lastLevel) p += 4;
        lastLevel = level;
        close(); para = { text: line.slice(m[0].length), width: width - 22 * (1 + level), fs: FS, lh: LH, extra: 0, item: true };
        continue;
      }
      if (/^>\s?/.test(line)) {
        close(); lastLevel = null; para = { text: line.replace(/^>\s?/, ""), width: width - 24, fs: FS, lh: LH, extra: 0 };
        continue;
      }
      // A line under a rule-like or setext marker is left as its own line (rare in chat; counted high).
      if (/^\s*([-*_=])(\s*\1){2,}\s*$/.test(line)) { close(); lastLevel = null; p += LH; continue; }
      if (para) { para.text += " " + line.trim(); continue; }
      lastLevel = null; para = { text: line, width, fs: FS, lh: LH, extra: 0 };
    }
  }
  close();
  return Math.ceil(h + p + (p > 0 ? END_PAD : 0));
}

export { tableCells };

// One line of text, in px at font size fs (markdown marks left out; leans wide like wrapLines).
export function textWidth(text, fs = 13.5) {
  let w = 0;
  for (const ch of String(text).replace(/\*\*|__|`/g, "")) w += isWide(ch.charCodeAt(0)) ? fs : fs * 0.58;
  return Math.ceil(w);
}

// Column widths for a table `width` px wide (1 px frame each side), from the cells' content, like an
// HTML table: each column wants its longest cell on one line (+24 px padding). If all fit, the space
// left is shared out in proportion. If not, the columns that fit their share keep their width and the
// rest split what is left in proportion to what they want, never under MIN_COL.
const MIN_COL = 64;
export function colWidths(width, rows, n) {
  const inner = Math.max(n * MIN_COL, width - 2);
  const want = Array.from({ length: n }, (_v, c) => Math.max(MIN_COL, ...rows.map((r) => textWidth(r[c] ?? "") + 24 + 4)));
  const total = want.reduce((a, b) => a + b, 0);
  let ws;
  if (total <= inner) ws = want.map((w) => w + (inner - total) * w / total);
  else {
    const fixed = new Set();
    for (;;) {
      const room = inner - [...fixed].reduce((a, c) => a + want[c], 0);
      const open = want.map((w, c) => c).filter((c) => !fixed.has(c));
      const wantOpen = open.reduce((a, c) => a + want[c], 0);
      const fits = open.filter((c) => want[c] <= room / open.length);
      if (!fits.length) {
        ws = want.map((w, c) => fixed.has(c) ? w : Math.max(MIN_COL, room * w / wantOpen));
        break;
      }
      fits.forEach((c) => fixed.add(c));
    }
  }
  const out = ws.map(Math.floor);
  out[n - 1] += Math.max(0, inner - out.reduce((a, b) => a + b, 0));
  return out;
}

export const columnWidth = (paneWidth) => Math.max(240, Math.min(READ_W, (paneWidth || 800) - GUTTER * 2));

// The height of a row. `open`: its tool chip shows the calls, or its note from another agent the whole text.
export function rowHeight(row, width, open = false) {
  switch (row.k) {
    case "user":
      if (row.from != null) return 40 + (open ? markdownHeight(row.text || "", width - 12) + 4 : 0);
      return markdownHeight(row.text || "", Math.floor(width * 0.78) - 28) + 20 + 16;
    case "assistant": return markdownHeight(row.text || "", width) + 16;
    case "tools": {
      const calls = (row.chips || []).reduce((n, c) => n + (c.calls || []).length, 0);
      return 40 + (open ? calls * 26 + 8 : 0);
    }
    case "sep": return 40;
    case "footer": return 30;
    case "status": return 34;
    case "perm": case "question": return 44;
    default: return 30;
  }
}

export function sizes(rows, width, open = new Set()) {
  return rows.map((r) => rowHeight(r, width, open.has(r.id)));
}
