// Agent markdown built as DOM nodes (never innerHTML: agent text is untrusted), after the Mac reading view
// (MarkdownView.swift): headings, paragraphs (line breaks kept), bullet/ordered/task lists (nested), > quotes,
// code fences (also indented inside list items), GFM tables, --- rules; inline `code`, **bold**, *italic*,
// ~~strike~~ and http(s) links. Anything else stays plain text.

export function markdown(src        )                   {
  const out = document.createDocumentFragment();
  budget = MAX_NODES;
  blocks(src.replace(/\r\n?/g, "\n").split("\n"), out);
  return out;
}

// Every pattern here runs in linear time on hostile lines (no two unbounded runs that can share characters).
// ([\s\S]*, not .*: a line may hold U+2028, which . does not match.)
const FENCE = /^ {0,3}(`{3,}|~{3,})([\s\S]*)$/;
const HEADING = /^ {0,3}(#{1,6})(?=[ \t]|$)/;
const RULE = /^ {0,3}([-*_])(?:\s*\1){2,}\s*$/;
const QUOTE = /^ {0,3}> ?/;
const ITEM = /^( *)([-*+]|\d{1,9}[.)])(\s+|$)([\s\S]*)$/;

// Quotes and lists nest by recursion; past this depth the rest is shown as plain text (hostile input cannot
// exhaust the stack).
const MAX_DEPTH = 16;
// Tables pad every row to the header: cap columns and cells, or 4 KB of '|' could build 250,000 cells.
const MAX_COLS = 64, MAX_CELLS = 10_000;
// Inline markup nests by recursion too; and a span (bold, link text, ...) is at most this long, so a scan for its
// closing marker stays short. Text longer than MAX_LINE is not scanned for inline markup at all.
const MAX_INLINE_DEPTH = 8, MAX_LINE = 20_000;
// Elements one message may build; past it the rest is plain text (many short paragraphs or table cells full of
// *a* would otherwise build hundreds of thousands of nodes).
const MAX_NODES = 20_000;
let budget = MAX_NODES;

// A code fence: backticks (whose info string has no backtick) or tildes. Returns [fence, lang].
function fence(line        )                          {
  const m = line.match(FENCE);
  if (!m || (m[1][0] === "`" && m[2].includes("`"))) return null;
  return [m[1], m[2].trim().split(/[ \t]/)[0]];
}
// The heading text without the optional closing #s.
function heading(line        )                          {
  const m = line.match(HEADING);
  if (!m) return null;
  const t = line.slice(m[0].length).trim();
  let end = t.length;
  while (end > 0 && t[end - 1] === "#") end--;   // a loop, not /#+$/ (quadratic on a long run of #)
  if (end < t.length && (end === 0 || t[end - 1] === " " || t[end - 1] === "\t")) return [m[1].length, t.slice(0, end).trim()];
  return [m[1].length, t];
}
// The delimiter row under a table header: cells of dashes with optional colons.
function delim(line        )          {
  const t = line.trim().replace(/^\|/, "").replace(/\|$/, "");
  return t.includes("-") && t.split("|").every((c) => /^:?-+:?$/.test(c.trim()));
}

const indentOf = (l        ) => l.length - l.trimStart().length;
const isTable = (lines          , i        ) => lines[i].includes("|") && i + 1 < lines.length && delim(lines[i + 1]);
const startsBlock = (lines          , i        ) =>
  !!fence(lines[i]) || HEADING.test(lines[i]) || RULE.test(lines[i]) || QUOTE.test(lines[i]) || ITEM.test(lines[i]) || isTable(lines, i);

function el                                       (tag   , ...kids                   )                           {
  budget--;
  const e = document.createElement(tag);
  e.append(...kids);
  return e;
}

function blocks(lines          , into            , depth = 0)       {
  if (depth > MAX_DEPTH) {
    into.append(el("p", lines.join("\n").trim()));
    return;
  }
  let i = 0;
  while (i < lines.length) {
    if (budget <= 0) {
      into.append(lines.slice(i).join("\n"));
      return;
    }
    const line = lines[i];
    if (!line.trim()) { i++; continue; }
    let f                         , h                         ;
    if ((f = fence(line))) {
      // The closing fence uses the same character, at least as long; the opening indent is taken off each line.
      const [mark, lang] = f, ind = indentOf(line), code           = [];
      const close = new RegExp(`^ {0,3}${mark[0] === "`" ? "`" : "~"}{${mark.length},}[ \\t]*$`);
      for (i++; i < lines.length && !close.test(lines[i]); i++) code.push(lines[i].slice(Math.min(ind, indentOf(lines[i]))));
      i++;
      const c = el("code", code.join("\n"));
      if (lang) c.dataset.lang = lang;
      into.append(el("pre", c));
    } else if ((h = heading(line))) {
      into.append(el(`h${h[0]}`        , spans(h[1])));
      i++;
    } else if (RULE.test(line)) {
      into.append(el("hr"));
      i++;
    } else if (QUOTE.test(line)) {
      const inner           = [];
      for (; i < lines.length && lines[i].trim() && (QUOTE.test(lines[i]) || !startsBlock(lines, i)); i++) inner.push(lines[i].replace(QUOTE, ""));
      const q = el("blockquote");
      blocks(inner, q, depth + 1);
      into.append(q);
    } else if (isTable(lines, i)) {
      i = table(lines, i, into);
    } else if (ITEM.test(line)) {
      i = list(lines, i, into, depth);
    } else {
      const para           = [];
      for (; i < lines.length && lines[i].trim() && (para.length === 0 || !startsBlock(lines, i)); i++) para.push(lines[i].trim());
      into.append(el("p", spans(para.join("\n"))));
    }
  }
}

// One list: items of the same kind (bullet or ordered) at this indent. An item owns the lines indented past its
// marker, blank lines inside it, and lazy continuation text; its content is parsed as blocks again (nesting).
function list(lines          , i        , into            , depth        )         {
  const first = lines[i].match(ITEM) ;
  const base = first[1].length, ordered = /\d/.test(first[2]);
  const ul = ordered ? el("ol") : el("ul");
  if (ordered && parseInt(first[2]) !== 1) ul.setAttribute("start", String(parseInt(first[2])));
  while (i < lines.length && budget > 0) {   // out of budget: blocks() shows the rest as text
    const m = lines[i].match(ITEM);
    if (!m || m[1].length < base || m[1].length > base + 3 || /\d/.test(m[2]) !== ordered) break;
    const content = m[3] ? base + m[2].length + Math.min(m[3].length, 4) : base + m[2].length + 1;
    const body = [m[4]];
    let lastBlank = false;
    for (i++; i < lines.length; i++) {
      const l = lines[i];
      if (!l.trim()) { body.push(""); lastBlank = true; continue; }
      const ind = indentOf(l);
      if (ind >= content) body.push(l.slice(content));
      else if (!lastBlank && ind > base && !ITEM.test(l)) body.push(l.trim());
      else if (!lastBlank && !startsBlock(lines, i)) body.push(l.trim());   // lazy continuation
      else if (ind > base && ITEM.test(l)) body.push(l.slice(Math.min(ind, content)));   // a nested list, under-indented
      else break;
      lastBlank = false;
    }
    while (body.length && !body[body.length - 1].trim()) body.pop();
    if (!body.length) body.push("");   // an empty item ('-' or '1.' alone)
    const li = el("li");
    const task = body[0].match(/^\[([ xX])\]\s+/);
    if (task) {
      li.className = "task";
      li.append(el("span", task[1] === " " ? "☐" : "☑"));
      body[0] = body[0].slice(task[0].length);
    }
    blocks(body, li, depth + 1);
    ul.append(li);
    // A blank line then something that is not an item of this list ends it.
    if (i < lines.length && !lines[i].trim()) {
      let j = i;
      while (j < lines.length && !lines[j].trim()) j++;
      const n = j < lines.length ? lines[j].match(ITEM) : null;
      if (!n || n[1].length !== base || /\d/.test(n[2]) !== ordered) break;
      i = j;
    }
  }
  into.append(ul);
  return i;
}

// A GFM table: header row, delimiter row (alignment), body rows until a blank line or a line without a pipe.
// Wrapped in its own box that scrolls sideways when the table is wider than the screen.
function table(lines          , i        , into            )         {
  const cells = (l        ) => l.trim().replace(/^\|/, "").replace(/(?<!\\)\|$/, "").split(/(?<!\\)\|/).map((c) => c.trim().replace(/\\\|/g, "|"));
  const start = i, head = cells(lines[i]);
  let end = i + 2;
  while (end < lines.length && lines[end].trim() && lines[end].includes("|")) end++;
  // Too big to be a table anyone reads on a phone: show the lines as they are.
  // The table's own elements (cells, rows, wrappers) are held back from the budget while it is built, so the
  // inline markup in early cells cannot use up what the later cells need.
  const n = head.length * (end - start - 1), own = n + (end - start - 1) + 4;
  if (head.length > MAX_COLS || n > MAX_CELLS || own > budget) {
    into.append(el("p", lines.slice(start, end).join("\n")));
    return end;
  }
  const aligns = cells(lines[i + 1]).map((d) => (d.startsWith(":") && d.endsWith(":") ? "center" : d.endsWith(":") ? "right" : ""));
  const body = lines.slice(i + 2, end).map(cells);
  i = end;
  budget -= own;
  // A column whose cells are all short (like "#") keeps its own width; the others get a readable minimum.
  const short = head.map((h, c) => [h, ...body.map((r) => r[c] ?? "")].every((v) => [...v].length <= 6));
  const row = (vals          , tag             ) => {
    const tr = el("tr");
    head.forEach((_, c) => {
      const cell = el(tag, spans(vals[c] ?? ""));
      if (aligns[c]) cell.style.textAlign = aligns[c];
      if (short[c]) cell.className = "short";
      tr.append(cell);
    });
    return tr;
  };
  const tbody = el("tbody");
  for (const r of body) tbody.append(row(r, "td"));   // one by one: spreading many rows overflows the call stack
  const wrap = el("div", el("table", el("thead", row(head, "th")), tbody));
  wrap.className = "table-wrap";
  into.append(wrap);
  budget += own;
  return i;
}

// Every span is bounded in length, so the scan does at most a few hundred steps per character.
const INLINE = /`([^`\n]{1,1000})`|\*\*([^\n]{1,300}?)\*\*|__([^\n]{1,300}?)__(?!\w)|~~([^\n]{1,300}?)~~|\*([^*\s](?:[^*\n]{0,300}?[^*\s])?)\*|(?<!\w)_([^_\s](?:[^_\n]{0,300}?[^_\s])?)_(?!\w)|\[([^\]\n]{1,300})\]\((https?:\/\/[^)\s]{1,2000})\)/g;

export function inline(text        )                   {
  budget = MAX_NODES;
  return spans(text);
}
function spans(text        , depth = 0)                   {
  const out = document.createDocumentFragment();
  // Room for a new element plus the ones around it, built after this returns: its depth's spans and the block.
  const room = () => budget >= depth + 2;
  if (depth > MAX_INLINE_DEPTH || text.length > MAX_LINE || !room()) {
    out.append(text);
    return out;
  }
  const re = new RegExp(INLINE);
  let at = 0;
  for (const m of text.matchAll(re)) {
    if (!room()) break;
    out.append(text.slice(at, m.index));
    if (m[1] !== undefined) out.append(el("code", m[1]));
    else if (m[2] !== undefined || m[3] !== undefined) out.append(el("strong", spans(m[2] ?? m[3], depth + 1)));
    else if (m[4] !== undefined) out.append(el("del", spans(m[4], depth + 1)));
    else if (m[5] !== undefined || m[6] !== undefined) out.append(el("em", spans(m[5] ?? m[6], depth + 1)));
    else {
      const a = el("a", m[7]);
      a.href = m[8];
      a.target = "_blank";
      a.rel = "noopener noreferrer";
      out.append(a);
    }
    at = (m.index ?? 0) + m[0].length;
  }
  out.append(text.slice(at));
  return out;
}
