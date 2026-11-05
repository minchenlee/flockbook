// Markdown with our own code boxes and tables: herdr's TextView draws them square and takes no CSS,
// so the text is split into blocks; code fences and tables are drawn here (rounded), the rest stays
// TextView markdown. Heights stay as estimate.js guesses them (same line heights).
import { div } from "gpui-kit";
import { v_flex, h_flex, TextView } from "gpui-base";
import { tableCells, colWidths } from "./estimate.js";

export { tableCells };

const FENCE = /^\s*(```|~~~)\s*([\w+#.-]*)\s*$/;
const SEP = /^\s*\|?\s*:?-{2,}:?\s*(\|\s*:?-{2,}:?\s*)*\|?\s*$/;

const alignOf = (cell) => {
  const c = cell.trim();
  return c.startsWith(":") && c.endsWith(":") ? "center" : c.endsWith(":") ? "right" : "left";
};

// [{k:"md", text}, {k:"code", lang, text}, {k:"table", head, align, rows}] in order.
export function splitBlocks(text) {
  const lines = String(text ?? "").split("\n");
  const out = []; let md = [];
  const flush = () => { if (md.length && md.join("\n").trim()) out.push({ k: "md", text: md.join("\n") }); md = []; };
  for (let i = 0; i < lines.length; i++) {
    const f = FENCE.exec(lines[i]);
    if (f) {
      flush();
      const body = []; i++;
      while (i < lines.length && !(FENCE.exec(lines[i]) && lines[i].trim().startsWith(f[1]))) body.push(lines[i++]);
      out.push({ k: "code", lang: f[2] || "", text: body.join("\n") });
      continue;
    }
    if (/^\s*\|/.test(lines[i]) && i + 1 < lines.length && SEP.test(lines[i + 1])) {
      flush();
      const head = tableCells(lines[i]);
      const align = tableCells(lines[i + 1]).map(alignOf);
      const rows = []; i += 2;
      while (i < lines.length && /^\s*\|/.test(lines[i])) rows.push(tableCells(lines[i++]));
      i--;
      out.push({ k: "table", head, align, rows });
      continue;
    }
    md.push(lines[i]);
  }
  flush();
  return out;
}

// The block as written, for Copy: CRs dropped, normal spaces and tabs.
export function codeCopyText(b) {
  return b.text.split("\n").map((l) => l.replace(/\r$/, "")).join("\n");
}

// The block as one markdown paragraph, so a drag selects across its lines: every ASCII punctuation
// escaped (drawn as text, not markup), leading spaces as no-break spaces (markdown drops them), tabs
// as 4, an empty line as one no-break space (else the paragraph ends), lines joined by hard breaks.
const NBSP = "\u00a0";
export function codeMarkdown(text) {
  return codeCopyText({ text }).split("\n").map((l) => {
    const e = l.replace(/\t/g, "    ").replace(/[!-/:-@[-`{-~]/g, "\\$&");
    return e.replace(/^ +/, (s) => NBSP.repeat(s.length)) || NBSP;
  }).join("\\\n");
}

function codeBox(key, b, t) {
  return v_flex().w_full().my(6).px(14).py(10).rounded(12).bg(t.raised).border(1).border_color(t.border)
    .font_family("Menlo").text_size(13).text_color(t.foreground)
    .child(TextView.markdown(key, codeMarkdown(b.text)).selectable(true));
}

// Cells get a fixed width: a TextView in a flexible cell keeps its full one-line width and spills
// out of the table. Widths follow the content (colWidths); estimate.js guesses the row heights from
// the same widths.
function table(id, b, t, width) {
  const n = Math.max(b.head.length, ...b.rows.map((r) => r.length));
  const ws = colWidths(width, [b.head, ...b.rows], n);
  const cell = (key, text, c, head) => div().w(ws[c]).flex_none().overflow_hidden().px(12).py(8)
    .when(b.align[c] === "right", (el) => el.text_right()).when(b.align[c] === "center", (el) => el.text_center())
    .when(head, (el) => el.font_semibold())
    .child(TextView.markdown(key, text ?? "").selectable(true));
  const row = (cells, r, head) => h_flex().w_full().items_start()
    .when(head, (el) => el.bg(t.raised).rounded_t(11))
    .when(!head, (el) => el.border_t(1).border_color(t.border))
    .children(Array.from({ length: n }, (_v, c) => cell(`${id}-c${r}-${c}`, cells[c], c, head)));
  return v_flex().w_full().my(6).rounded(12).border(1).border_color(t.border).text_size(13.5).overflow_hidden()
    .child(row(b.head, "h", true))
    .children(b.rows.map((r, i) => row(r, i, false)));
}

// The whole text: one element. `id` keeps the TextView ids unique per row; `width` is the text's
// width in px (tables need it).
export function markdownView(id, text, t, width) {
  const blocks = splitBlocks(text);
  if (blocks.length === 1 && blocks[0].k === "md") return TextView.markdown(`md-${id}`, blocks[0].text).selectable(true);
  return v_flex().w_full().children(blocks.map((b, i) =>
    b.k === "code" ? codeBox(`md-${id}-k${i}`, b, t)
      : b.k === "table" ? table(`md-${id}-t${i}`, b, t, width)
        : TextView.markdown(`md-${id}-${i}`, b.text).selectable(true)));
}
