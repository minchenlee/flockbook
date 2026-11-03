// Row heights for the virtual list, guessed from the text (the list needs them before layout).
// Body text: 14 px, 22 px per line. Over-guessing leaves a gap; under-guessing cuts the row, so the
// guesses lean high: wide Latin letters, and 8% on top for word wrap.
export const READ_W = 760;  // the reading column's width at most
export const GUTTER = 24;   // each side of the column

const isWide = (c) => c >= 0x1100 && (c <= 0x115f || (c >= 0x2e80 && c <= 0xa4cf) || (c >= 0xac00 && c <= 0xd7a3) ||
  (c >= 0xf900 && c <= 0xfaff) || (c >= 0xfe30 && c <= 0xfe4f) || (c >= 0xff00 && c <= 0xff60) || (c >= 0xffe0 && c <= 0xffe6));

export function wrapLines(text, width, fs = 14) {
  let w = 0, n = 1;
  for (let i = 0; i < text.length; i++) {
    const cw = isWide(text.charCodeAt(i)) ? fs : fs * 0.58;
    if (w + cw > width) { n++; w = cw; } else w += cw;
  }
  return n;
}

export function markdownHeight(text, width) {
  const LH = 22, CODE_LH = 19;
  let h = 0, inCode = false, prevBlank = true, inTable = false, tableCols = 1;
  for (const raw of String(text).split("\n")) {
    const line = raw.replace(/\t/g, "    ");
    if (/^\s*```/.test(line)) { h += inCode ? 16 : 34; inCode = !inCode; continue; }
    if (inCode) { h += CODE_LH * Math.max(1, wrapLines(line, width - 24, 13)); continue; }
    if (!line.trim()) { if (!prevBlank) h += 10; prevBlank = true; inTable = false; continue; }
    prevBlank = false;
    let m;
    if ((m = /^(#{1,6})\s/.exec(line))) { const size = [26, 22, 19, 17, 15, 14][m[1].length - 1]; h += wrapLines(line, width, size) * size * 1.4 + 12; continue; }
    if (/^\s*\|/.test(line)) {
      // mdblocks.js draws tables with fixed-width cells (cellWidth), 8 px padding top and bottom, 1 px
      // lines; a row is as tall as its longest cell. The frame adds 6 px above and below.
      if (!inTable) { h += 14; inTable = true; tableCols = line.split("|").length - 2 || 1; }
      if (!/^\s*\|[\s:|-]+\|?\s*$/.test(line)) {
        const cw = cellWidth(width, tableCols) - 24;
        const most = Math.max(1, ...line.split("|").slice(1, -1).map((c) => wrapLines(c.trim(), cw, 13.5)));
        h += 17 + most * 21;
      }
      continue;
    }
    inTable = false;
    if ((m = /^(\s*)([-*+]|\d+\.)\s/.exec(line))) { h += wrapLines(line, width - 24 - m[1].length * 8) * LH + 6; continue; }
    if (/^>\s?/.test(line)) { h += wrapLines(line, width - 24) * LH; continue; }
    h += wrapLines(line, width) * LH;
  }
  if (inCode) h += 16;
  return Math.ceil(h * 1.08);
}

// A table cell's width: the column split evenly (the table's 1 px frame left and right taken off).
export const cellWidth = (width, n) => Math.max(40, Math.floor((width - 2) / Math.max(1, n)));

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
