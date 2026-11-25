// Markdown with our own code boxes and tables: herdr's TextView draws them square and takes no CSS,
// so the text is split into blocks (blocks.js, the same split estimate.js counts); code fences and tables
// are drawn here (rounded), the rest stays TextView markdown.
import { div } from "gpui-kit";
import { v_flex, h_flex, TextView } from "gpui-base";
import { colWidths, markdownHeight } from "./estimate.js";
import { splitBlocks, tableCells } from "./blocks.js";

export { splitBlocks, tableCells };

// Links (Resources P0). The helper finds the targets in an assistant text and sends their places as the row's
// `links` ([{start, end, res, block}], UTF-16 offsets into `text`; docs/protocol.md). The chat draws them as Markdown
// links, and TextView opens them on a click (list rows cannot have handlers, so not cx.open_url). Only http(s):
// a file (`doc`) has no way to open from a mod yet and stays text. Pure: tests open nothing.
const WEB = new Set(["artifact", "pr", "issue", "ci", "link", "local"]);

// What a click opens: the target as written, with a scheme when the helper found it without one
// ("localhost:3000", "github.com/o/r/pull/3"). Null when it does not open.
export function linkHref(link, target) {
  if (!WEB.has(link?.res)) return null;
  const s = String(target ?? "");
  if (/^https?:\/\/[^\s/?#]/i.test(s)) return s;
  if (/^[a-z][a-z0-9+.-]*:\/\//i.test(s) || /^[a-z]+:[^0-9]/i.test(s)) return null;
  return (link.res === "local" ? "http://" : "https://") + s;
}

// The text with each link's range written as [visible](href). The visible text stays the same (markup in it
// escaped). Only a link whose `block` is `offset` is written: the helper set it where both the whole reply and this
// block read the range as plain text (not code, a link, a picture or HTML; mdsplit.rs). A link without `block` (an
// older helper, a cached row) stays text. Bad, overlapping or unknown ranges are skipped. `offset`: where `text`
// starts in the row's text (a block of it, from splitBlocks).
export function linkedMarkdown(text, links, offset = 0) {
  const s = String(text ?? "");
  if (!Array.isArray(links) || !links.length) return s;
  const ls = links.filter((l) => Number.isInteger(l?.block) && l.block === offset)
    .map((l) => ({ ...l, start: l.start - offset, end: l.end - offset }))
    .filter((l) => Number.isInteger(l.start) && Number.isInteger(l.end) && l.start >= 0 && l.end <= s.length && l.start < l.end)
    .sort((a, b) => a.start - b.start);
  let out = "", at = 0;
  for (const l of ls) {
    if (l.start < at) continue;
    const seen = s.slice(l.start, l.end), href = linkHref(l, seen);
    if (!href) continue;
    // Backslash before & and \ so an entity ("&amp;") or an escape is not read in the address.
    const raw = href.replace(/[\\&]/g, (c) => "\\" + c);
    const dest = /[()\s<>]/.test(href) ? `<${raw}>` : raw;
    out += s.slice(at, l.start) + "[" + seen.replace(/[\\[\]*_~`<>]/g, (c) => "\\" + c) + "](" + dest + ")";
    at = l.end;
  }
  return out + s.slice(at);
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

// The code box under `y` (px from the text's top, at `width`), as written for Copy; null when `y` is not
// on one. Box places are the estimate's, the same guess the row heights use.
export function codeBlockAt(text, width, y, k = 1, zoom = 1) {
  const spans = [];
  markdownHeight(String(text ?? ""), width, spans, k, zoom);
  const s = spans.find((b) => y >= b.top && y <= b.bottom);
  return s ? s.lines.join("\n") : null;
}

function codeBox(key, b, t, k) {
  return v_flex().w_full().my(6).px(14).py(10).rounded(12).bg(t.raised).border(1).border_color(t.border)
    .font_family("Menlo").text_size(13 * k).text_color(t.foreground)
    .child(TextView.markdown(key, codeMarkdown(b.text)).selectable(true));
}

// Cells get a fixed width: a TextView in a flexible cell keeps its full one-line width and spills
// out of the table. Widths follow the content (colWidths); estimate.js guesses the row heights from
// the same widths.
function table(id, b, t, width, k) {
  const n = Math.max(b.head.length, ...b.rows.map((r) => r.length));
  const ws = colWidths(width, [b.head, ...b.rows], n, k);
  const cell = (key, text, c, head) => div().w(ws[c]).flex_none().overflow_hidden().px(12).py(8)
    .when(b.align[c] === "right", (el) => el.text_right()).when(b.align[c] === "center", (el) => el.text_center())
    .when(head, (el) => el.font_semibold())
    .child(TextView.markdown(key, text ?? "").selectable(true));
  const row = (cells, r, head) => h_flex().w_full().items_start()
    .when(head, (el) => el.bg(t.raised).rounded_t(11))
    .when(!head, (el) => el.border_t(1).border_color(t.border))
    .children(Array.from({ length: n }, (_v, c) => cell(`${id}-c${r}-${c}`, cells[c], c, head)));
  return v_flex().w_full().my(6).rounded(12).border(1).border_color(t.border).text_size(13.5 * k).overflow_hidden()
    .child(row(b.head, "h", true))
    .children(b.rows.map((r, i) => row(r, i, false)));
}

// The whole text: one element. `id` keeps the TextView ids unique per row; `width` is the text's
// width in px (tables need it). A text block's id is always md-<id>-<i>, one block or many: a message that
// grows a code box while the agent writes keeps its first view (and the selection in it).
// `k`: the chat's text size over the default 16 px (settings: text_size). At 1 nothing sets a size.
// `links`: the row's links (assistant rows), drawn in prose blocks only. Heights and Copy use the text as it is.
export function markdownView(id, text, t, width, k = 1, links = null) {
  const blocks = splitBlocks(text);
  const prose = (key, md) => {
    const view = TextView.markdown(key, md).selectable(true);
    return k === 1 ? view : view.text_size(16 * k);
  };
  const linked = (b) => linkedMarkdown(b.text, links, b.start ?? 0);
  if (blocks.length === 1 && blocks[0].k === "md") return prose(`md-${id}-0`, linked(blocks[0]));
  return v_flex().w_full().children(blocks.map((b, i) =>
    b.k === "code" ? codeBox(`md-${id}-k${i}`, b, t, k)
      : b.k === "table" ? table(`md-${id}-t${i}`, b, t, width, k)
        : prose(`md-${id}-${i}`, linked(b))));
}
