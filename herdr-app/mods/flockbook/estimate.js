// Row heights for the virtual list, guessed from the text (the list needs them before layout).
// Body text as TextView draws it (measured in the app): 16 px (1rem), 26 px per line (gpui's line
// height, about 1.618 × the size). Over-guessing leaves a gap; under-guessing cuts the row, so the
// guesses lean high: prose wraps by unit (a Latin word or an inline code span does not break, closing
// full-width punctuation stays with the character before it), letters are counted wide, and 8% goes on
// top of the prose. Code boxes and tables get their drawn height, with no 8%: on a long answer full of
// code the 8% left a big gap.
export const READ_W = 760;  // the reading column's width at most
export const GUTTER = 24;   // each side of the column

const isWide = (c) => c >= 0x1100 && (c <= 0x115f || (c >= 0x2e80 && c <= 0xa4cf) || (c >= 0xac00 && c <= 0xd7a3) ||
  (c >= 0xf900 && c <= 0xfaff) || (c >= 0xfe30 && c <= 0xfe4f) || (c >= 0xff00 && c <= 0xff60) || (c >= 0xffe0 && c <= 0xffe6));

// Body text size and line height, in px.
export const FS = 16, LH = 26;
// Latin letters in prose, in em: measured about 0.47 em (spaces included); a little more.
const LATIN_EM = 0.5;
// Inline code: Menlo at 14 px, 0.6 em a character, 4 px padding each side.
const CODE_CH = 14 * 0.6, CODE_PAD = 8;
// Full-width punctuation that may not start a line: it goes down with the character before it.
const NO_START = new Set("，。、；：？！）」』】〉》〕｝％…—～·");

// Lines of prose drawn `width` px wide at `fs` px. Each line keeps one character of slack: line
// breaking (no-start punctuation, a word moved down) can end a line a character early.
export function proseLines(text, full, fs = FS) {
  const width = Math.max(fs, full - fs);
  const units = String(text).match(/`[^`]+`|[A-Za-z0-9_\-+./:@#%&=?!'"$<>~^*]+|\s+|./gsu) ?? [];
  const wide = (u) => isWide(u.codePointAt(0));
  const size = (u) => u[0] === "`" && u.length > 1 ? (u.length - 2) * CODE_CH + CODE_PAD
    : /^\s+$/.test(u) ? fs * 0.3 * u.length
      : wide(u) ? fs : u.length * fs * LATIN_EM;
  let n = 1, w = 0;
  for (let i = 0; i < units.length; i++) {
    let u = units[i], uw = size(u);
    // Keep a run of no-start punctuation with the unit before it.
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

export function markdownHeight(text, width) {
  const CODE_LH = 19;
  let h = 0, p = 0, inCode = false, prevBlank = true;  // h: drawn exactly; p: prose, wraps
  // A blank line is a 10 px paragraph gap, but not next to a code box or table: their margin is the gap.
  let gap = 0;
  const box = () => { h -= gap; gap = 0; };
  const lines = String(text).split("\n");
  for (let li = 0; li < lines.length; li++) {
    const line = lines[li].replace(/\t/g, "    ");
    // A code box as mdblocks.js draws it: 6 px margin, 10 px padding and a 1 px border, each side.
    if (/^\s*```/.test(line)) { if (!inCode) { box(); h += 34; } else prevBlank = true; inCode = !inCode; continue; }
    if (inCode) { h += CODE_LH * Math.max(1, wrapLines(line, width - 30, 13)); continue; }
    if (!line.trim()) { if (!prevBlank) { h += 10; gap = 10; } prevBlank = true; continue; }
    prevBlank = false; gap = 0;
    let m;
    if ((m = /^(#{1,6})\s/.exec(line))) { const size = [30, 25, 22, 20, 17, 16][m[1].length - 1]; p += proseLines(line, width, size) * size * 1.62 + 12; continue; }
    if (/^\s*\|/.test(line) && li + 1 < lines.length && TABLE_SEP.test(lines[li + 1])) {
      // A table as mdblocks.js draws it: 6 px above and below, a 1 px frame; each row 8 px padding top
      // and bottom plus a 1 px line, as tall as its tallest cell at the colWidths widths.
      const rows = [tableCells(line)];
      let j = li + 2;
      while (j < lines.length && /^\s*\|/.test(lines[j])) rows.push(tableCells(lines[j++]));
      const n = Math.max(...rows.map((r) => r.length));
      const ws = colWidths(width, rows, n);
      box(); h += 14;
      for (const r of rows) h += 17 + 21 * Math.max(1, ...ws.map((w, c) => wrapLines(r[c] ?? "", w - 24, 13.5)));
      li = j - 1; prevBlank = true;
      continue;
    }
    if ((m = /^(\s*)([-*+]|\d+\.)\s/.exec(line))) { p += proseLines(line.slice(m[0].length), width - 24 - m[1].length * 8) * LH + 6; continue; }
    if (/^>\s?/.test(line)) { p += proseLines(line.replace(/^>\s?/, ""), width - 24) * LH; continue; }
    p += proseLines(line, width) * LH;
  }
  return Math.ceil(h + p * 1.08);
}

const TABLE_SEP = /^\s*\|?\s*:?-{2,}:?\s*(\|\s*:?-{2,}:?\s*)*\|?\s*$/;

// "| a | b |" → ["a", "b"]. An escaped pipe and pipes inside `code` stay in the cell.
export function tableCells(line) {
  let s = line.trim();
  if (s.startsWith("|")) s = s.slice(1);
  if (s.endsWith("|") && !s.endsWith("\\|")) s = s.slice(0, -1);
  const out = []; let cur = "", code = false;
  for (let i = 0; i < s.length; i++) {
    const c = s[i];
    if (c === "\\" && s[i + 1] === "|") { cur += "|"; i++; continue; }
    if (c === "`") code = !code;
    if (c === "|" && !code) { out.push(cur.trim()); cur = ""; continue; }
    cur += c;
  }
  out.push(cur.trim());
  return out;
}

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

// The height of a row. `open`: its tool chip shows the calls.
export function rowHeight(row, width, open = false) {
  switch (row.k) {
    case "user": return markdownHeight(row.text || "", Math.floor(width * 0.78) - 28) + 20 + 16;
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
