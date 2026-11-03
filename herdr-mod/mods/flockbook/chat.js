// The "chat" view: the shown agent's conversation, from the sidecar's `feed` (docs/protocol.md).
// The list is a virtual list with guessed heights (estimate.js): `list` measures rows itself, but it
// cannot be scrolled from code, and the chat must open at the newest message. Every row shows in full.
import { div, svg } from "gpui-kit";
import { v_flex, h_flex, TextView, Scrollbar, v_virtual_list, VirtualListScrollHandle } from "gpui-base";
import { theme, run } from "herdr";
import { currentAgent, icon, SPIN_MS, FADE, shownScreen, watchEdge, noteWindowHeight } from "./state.js";
import { Appear } from "./motion.js";
import { now, record, flush, loadTiming } from "./timing.js";
import * as sidecar from "./sidecar.js";
import { emptyFeed, applyFeed, applyOlder, olderFrom } from "./feedstate.js";
import { GUTTER, columnWidth, rowHeight } from "./estimate.js";
import { Composer } from "./composer.js";
import { markdownView } from "./mdblocks.js";
import { Cards, wanted } from "./cards.js";
import { startTop, overBar, BAR_H } from "./top.js";

export const POLL_MS = 2000;
const CHECK_MS = 400;

// The chip's SF Symbols name, mapped to one of this mod's icons.
export function chipIcon(symbol) {
  const s = String(symbol || "");
  if (s.startsWith("terminal")) return icon("terminal");
  if (s.startsWith("doc") || s.includes("pencil")) return icon("doc");
  if (s.includes("magnifyingglass")) return icon("search");
  return icon("tool");
}

// ⌘⌥↑ / ⌘⌥↓ (any order of the modifiers): the previous or next space.
export function spaceKey(e) {
  const m = e?.modifiers ?? {};
  if (!(m.platform || m.cmd) || !m.alt || m.shift || m.control) return null;
  return e.key === "up" ? "space.prev" : e.key === "down" ? "space.next" : null;
}

export class ChatView {
  constructor(cx) {
    this.scroll = VirtualListScrollHandle.new();
    this.feeds = new Map();      // address → feed state (emptyFeed)
    this.address = null;         // the agent shown
    this.width = 0;              // the pane's width, from on_size_change
    this.follow = true;          // keep the newest row on screen
    this.open = new Set();       // tool rows that show their calls
    this.heights = new WeakMap(); // row → { w, open, h }
    this.busy = false;
    this.lastPoll = 0;
    this.composer = new Composer(cx);
    this.top = {}; startTop(cx, this.top);
    this.appear = new Appear();
    this.cards = new Cards(cx);
    this.cards.canFocus = () => this.composer?.empty !== false;
    this.switchTo(currentAgent()?.address ?? null);
    cx.spawn((cx) => this.poll(cx));
    cx.timer.every(CHECK_MS, (cx) => this.tick(cx));
    cx.spawn(async () => { if (await loadTiming()) cx.timer.every(1000, () => flush()); });
  }

  feed() { return this.address ? this.feeds.get(this.address) : null; }

  // Every chat opens at its newest message, also one read before.
  switchTo(address) {
    this.address = address; this.cards.shown = address;
    if (address && !this.feeds.has(address)) this.feeds.set(address, emptyFeed(address));
    this.follow = true; this.open = new Set();
    this.toNewest();
  }

  tick(cx) {
    const a = currentAgent()?.address ?? null;
    if (a !== this.address) { this.switchTo(a); this.lastPoll = 0; cx.notify(); }
    // Coming back to the chat screen (from Overview, Needs you, a shell tab) also opens at the end.
    const screen = shownScreen();
    if (screen === "chat" && this.screen !== "chat") { this.follow = true; this.toNewest(); cx.notify(); }
    this.screen = screen;
    cx.spawn((cx) => this.composer.pollStatus(cx));
    const agent = currentAgent();
    cx.spawn((cx) => this.cards.poll(cx, this.address, wanted(agent, this.feed()?.rows)));
    if (this.wantOlder) { this.wantOlder = false; cx.spawn((cx) => this.loadOlder(cx)); }
    if (Date.now() - this.lastPoll >= POLL_MS) return this.poll(cx);
  }

  // The first row is on screen and the log goes back further: read the 2 MB before and put it on
  // top, keeping the same row where it was.
  async loadOlder(cx) {
    const f = this.feed();
    const before = f ? olderFrom(f) : null;
    if (before === null || this.olderBusy) return;
    this.olderBusy = true; cx.notify();
    try {
      const t0 = now();
      const answer = await sidecar.older(f.address, before, f.path);
      record("feed.older", now() - t0);
      const added = applyOlder(f, answer, before);
      if (added && f === this.feed()) this.scroll.scroll_to_item(added, "top");
    } finally { this.olderBusy = false; cx.notify(); }
  }

  // One `feed` for the shown agent. Notifies only when rows or the error changed.
  async poll(cx) {
    const f = this.feed();
    if (!f || this.busy) return;
    this.busy = true; this.lastPoll = Date.now();
    try {
      const t0 = now();
      const answer = await sidecar.feed(f.address, f.cursor);
      const t1 = now();
      const at = applyFeed(f, answer);
      record("feed.exec", t1 - t0);
      record("feed.apply", now() - t1, at >= 0 ? 1 : 0);
      if (at >= 0 && f === this.feed()) {
        if (this.follow) this.toNewest();
        cx.notify();
      }
    } finally { this.busy = false; }
  }

  toNewest() {
    const n = this.feed()?.rows.length ?? 0;
    if (n) this.scroll.scroll_to_item(n - 1, "top");
  }

  colWidth() { return columnWidth(this.width); }

  height(row) {
    const w = this.colWidth(), open = this.open.has(row.id);
    const c = this.heights.get(row);
    if (c && c.w === w && c.open === open) return c.h;
    const h = rowHeight(row, w, open);
    this.heights.set(row, { w, open, h });
    return h;
  }

  // A row's content, without the list's fixed height. No handlers: list rows cannot have any.
  body(row, t) {
    const md = (text, w = this.colWidth()) => markdownView(row.id, text || "", t, w);
    switch (row.k) {
      case "user":
        return h_flex().w_full().justify_end().py(8)
          .child(v_flex().max_w(Math.floor(this.colWidth() * 0.78)).px(14).py(10).gap(4).rounded(16).bg(t.raised).text_color(t.foreground)
            .child(md(row.text, Math.floor(this.colWidth() * 0.78) - 28))
            .when(Boolean(row.images), (el) => el.child(div().text_size(11.5).text_color(t.muted).child(row.images === 1 ? "1 image" : `${row.images} images`))));
      case "assistant":
        return div().w_full().py(8).text_color(t.foreground).child(md(row.text));
      case "tools": return this.tools(row, t);
      case "sep":
        return h_flex().w_full().h(40).items_center().justify_center().text_size(11.5).text_color(t.dim).child(row.text || "");
      case "footer":
        return div().w_full().h(30).pt(2).text_size(11.5).text_color(t.dim).child(row.text || "");
      case "status":
        return h_flex().w_full().h(34).items_center().justify_center().text_size(12).text_color(t.muted).child(row.text || "");
      case "perm": {
        // A call without a result yet. Whether it waits for an approval is the card's job (cards.js).
        const what = row.text || row.perm?.title || row.perm?.tool || "a tool";
        const pending = row.perm?.state === "pending";
        return h_flex().w_full().h(44).items_center().child(h_flex().h(28).px(10).gap(6).items_center().rounded(14)
          .bg(t.chrome).border(1).border_color(t.border).text_size(12.5).text_color(t.muted)
          .child(pending ? svg(icon("spinner")).size(11).text_color(t.working).spin(SPIN_MS) : svg(icon("check")).size(12).text_color(t.done))
          .child(div().max_w(Math.floor(this.colWidth() * 0.8)).truncate().child(pending ? `Running · ${what}` : what)));
      }
      case "question":
        return this.note(t, row.question?.state === "pending" ? "The agent asks a question" : "Question answered");
      default:
        return div().h(30);
    }
  }

  note(t, text) {
    return h_flex().w_full().h(44).items_center().child(div().px(12).py(6).rounded(10).border(1).border_color(t.border)
      .text_size(12.5).text_color(t.muted).truncate().child(text));
  }

  tools(row, t) {
    const open = this.open.has(row.id);
    const chips = row.chips || [];
    const running = chips.some((c) => (c.calls || []).some((k) => !k.done));
    const box = v_flex().w_full().py(6).gap(4)
      .child(h_flex().gap(6).flex_wrap().children(chips.map((c) =>
        h_flex().h(28).px(10).gap(6).items_center().rounded(14).bg(t.chrome).border(1).border_color(t.border)
          .text_size(12.5).text_color(t.muted)
          .child(svg(chipIcon(c.symbol)).size(13).flex_none())
          .child(c.text || "")
          .when(running, (el) => el.child(svg(icon("spinner")).size(11).text_color(t.working).spin(SPIN_MS)))
          .child(div().text_color(t.dim).child(open ? "▴" : "▾")))));
    if (!open) return box;
    const calls = chips.flatMap((c) => c.calls || []);
    return box.child(v_flex().pl(12).gap(0).children(calls.map((k) => h_flex().h(26).gap(8).items_center().text_size(12)
      .child(div().flex_none().text_color(t.foreground).child(k.name || "tool"))
      .child(div().flex_1().min_w(0).truncate().text_color(t.muted).child(k.summary || ""))
      .child(k.error ? div().flex_none().text_color(t.blocked).child("failed")
        : k.done ? svg(icon("check")).size(12).flex_none().text_color(t.done)
        : svg(icon("spinner")).size(11).flex_none().text_color(t.working).spin(SPIN_MS)))));
  }

  // One list row: the guessed height and the reading column.
  item(row, t) {
    const pad = Math.max(GUTTER, Math.floor((this.width - this.colWidth()) / 2));
    return div().h(this.height(row)).w_full().px(pad).overflow_hidden().child(this.body(row, t));
  }

  click(key, cx) {
    const row = this.feed()?.rows.find((r) => r.id === key);
    if (!row) return;
    if (row.k === "tools") {
      if (this.open.has(row.id)) this.open.delete(row.id); else this.open.add(row.id);
      cx.notify();
    }
  }

  // A failed `feed` in words people can act on; the sidecar's own message small below.
  problem(t, err) {
    const say = {
      "not-agent": "This terminal runs no agent.",
      "no-terminal": "This agent's terminal is gone.",
      "no-core": "herdr-core is not running.",
      core: "herdr-core did not answer. Trying again.",
      locate: "This agent's conversation was not found yet.",
      read: "This agent's conversation could not be read.",
      "no-sidecar": "The Flockbook helper is missing. Install it again.",
      protocol: "The Flockbook helper does not match this version. Install both again.",
    }[err.code] ?? "The conversation could not be loaded.";
    return v_flex().size_full().items_center().justify_center().gap(6).px(32)
      .child(div().text_size(13.5).text_color(t.foreground).child(say))
      .child(div().max_w(560).text_size(11.5).text_color(t.dim).truncate().child(err.message || ""));
  }

  center(t, text) {
    return v_flex().size_full().items_center().justify_center().px(24).text_size(13).text_color(t.muted).child(text);
  }

  list(t, f) {
    const rows = f.rows, n = rows.length;
    // The first row starts below the bar; the rest scroll up under it.
    const sizes = rows.map((r, i) => this.height(r) + (i === 0 ? BAR_H : 0));
    const box = v_flex().relative().flex_1().min_h(0)
      .on_scroll_wheel((e, cx) => {
        const up = e?.delta?.y > 0;
        if (this.follow && up) { this.follow = false; cx.notify(); }
        // Older rows load only when you scroll up while the first row is on screen.
        if (up && this.topSeen && f.start) this.wantOlder = true;
      })
      .child(v_virtual_list("chat", n, sizes, (i) => rows[i].id, (range) => {
        const t0 = now();
        if (range.end >= n) this.follow = true;
        this.topSeen = range.start === 0 && range.end < n;
        const out = [];
        for (let i = range.start; i < range.end; i++) out.push(i === 0 ? div().w_full().pt(BAR_H).child(this.item(rows[i], t)) : this.item(rows[i], t));
        record("chat.rows", now() - t0, range.end - range.start);
        return out;
      }).track_scroll(this.scroll).on_item_click((key, cx) => this.click(key, cx)))
      .child(Scrollbar.vertical("chat").absolute().inset_0());
    // A round ↓ that fades in while you read older messages and out when you are back at the end.
    if (n) {
      box.child(h_flex().absolute().bottom(12).left(0).right(0).justify_center()
        .child(div().id("jump").size(34).flex().items_center().justify_center().rounded(17)
          .bg(t.raised).border(1).border_color(t.border).shadow_sm().text_color(t.foreground)
          .opacity(this.follow ? 0 : 1).transition("opacity", FADE)
          .when(!this.follow, (el) => el.hover((s) => s.bg(t.hover)))
          .on_click((_e, cx) => { if (this.follow) return; this.follow = true; this.toNewest(); cx.notify(); })
          .child(svg(icon("arrow-down")).size(15))));
    }
    // While older rows load: a small note under the bar's tabs. Drawn only then: a hidden spinner still costs frames.
    if (this.olderBusy) box.child(h_flex().absolute().top(BAR_H + 6).left(0).right(0).justify_center()
      .child(h_flex().id("older").h(26).px(12).gap(6).items_center().rounded(13).bg(t.raised).border(1).border_color(t.border)
        .text_size(12).text_color(t.muted)
        .child(svg(icon("spinner")).size(11).spin(SPIN_MS)).child("Loading earlier messages")));
    return box;
  }

  render(cx) {
    const t0 = now();
    try { return this.draw(cx); } finally { record("chat.render", now() - t0); }
  }

  draw(cx) {
    const t = theme();
    const f = this.feed();
    let main;
    if (!f) main = this.center(t, "No agent to show. Start one with +.");
    else if (f.error && !f.rows.length) main = this.problem(t, f.error);
    else if (!f.loaded) main = this.center(t, "Loading…");
    else if (!f.rows.length) main = this.center(t, f.path ? "No messages yet." : "No conversation yet. Send the first message.");
    else main = this.list(t, f);
    const stop = (_e, cx) => cx.stop_propagation();
    return watchEdge(v_flex().id("root").relative().size_full().min_w(0).bg(t.background))
      .on_size_change((size, cx) => { noteWindowHeight(size); if (Math.abs(size.width - this.width) >= 1) { this.width = size.width; cx.notify(); } })
      .on_drop((e, cx) => this.composer.addFiles(e?.paths, cx))
      // The chat input keeps ⌘⌥↑/↓ from the app's keys, so the chat switches spaces itself.
      .on_key_down((e, cx) => { const go = spaceKey(e); if (go) { cx.stop_propagation?.(); run(go); } })
      // Another agent's chat fades in; a new card fades in and rises into place.
      // Presses on the content don't drag the window (the block has window_drag for the bar).
      .child(v_flex().flex_1().min_h(0).on_mouse_down("left", stop)
        .child(this.appear.wrap(main, "chat-body", this.address, cx).flex_1().min_h(0).flex().flex_col()))
      .when(Boolean(f) && f.error?.code !== "not-agent", (el) => {
        const card = this.cards.view(this.address, t, "chat-card");
        if (card) el.child(this.appear.wrap(h_flex().w_full().px(16).pt(8).justify_center().on_mouse_down("left", stop).child(card), "chat-card-in", `${this.address}:${this.cards.key?.(this.address) ?? ""}`, cx, 10));
        return el.child(div().w_full().on_mouse_down("left", stop).child(this.composer.render(currentAgent(), t)));
      })
      .child(overBar(t));
  }
}
