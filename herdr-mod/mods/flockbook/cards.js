// What an agent waits for: an approval ("Do you want to run …?") or a question, as a card with
// buttons. Read with the sidecar's `card`; answered with `approve` and `answer` (docs/protocol.md).
// Keys go out only on a button press or a key, never on a timer: that includes "Show all", which
// walks the picker's tabs (`card --all`).
import { div, svg } from "gpui-kit";
import { v_flex, h_flex, Textarea, TextareaState } from "gpui-base";
import { shared, setShared } from "herdr";
import { icon } from "./state.js";
import * as sidecar from "./sidecar.js";

export const CARD_MS = 1500;
const KEY_FULL = "flockbook.card.full"; // { address, card }: that full card

// Whether to read the card: herdr-app shows the agent as blocked, or the log ends with a call that
// waits (a pending perm or question row).
export function wanted(agent, rows) {
  if (!agent) return false;
  if (agent.status === "blocked") return true;
  const last = (rows || []).filter((r) => r.k === "perm" || r.k === "question").at(-1);
  return Boolean(last && (last.perm?.state === "pending" || last.question?.state === "pending"));
}

// The answers `answer --answers` takes, from the picks and "other" texts. Null while one is empty.
export function answersFor(question, picks, others) {
  const out = [];
  for (let i = 0; i < (question?.questions ?? []).length; i++) {
    const p = [...(picks.get(i) ?? [])].sort((a, b) => a - b);
    const other = (others.get(i) ?? "").trim();
    if (!p.length && !other) return null;
    out.push(other ? { picks: p, other } : { picks: p });
  }
  return out.length ? out : null;
}

export const MAX_QUESTIONS = 4;

// Whether question `qi` has a pick or an own answer.
const answered = (en, qi) => Boolean(en.picks.get(qi)?.size || (en.others.get(qi) ?? "").trim());

// A digit key's number (1-9), or null.
export function digit(e) {
  const m = e?.modifiers ?? {};
  if (m.platform || m.cmd || m.control || m.alt) return null;
  return /^[1-9]$/.test(e?.key ?? "") ? Number(e.key) : null;
}

export class Cards {
  // The "other answer" boxes are made here, once: a view may not make input state while it draws.
  // `cx`: for the card's own focus, so 1-9 and Enter reach it and not the reply box.
  constructor(cx) {
    this.focus = cx?.focus_handle?.() ?? null;
    this.canFocus = () => true;   // the chat says no while you type a reply
    this.byAgent = new Map();
    this.address = null;   // whose card the boxes belong to now
    this.others = Array.from({ length: MAX_QUESTIONS }, (_v, qi) => {
      const input = TextareaState.new({ placeholder: "Or type your own answer", rows: 1 });
      input.on("change", (_e, cx) => {
        const e = this.byAgent.get(this.address); if (!e) return;
        const was = Boolean((e.others.get(qi) ?? "").trim());
        e.others.set(qi, input.value());
        if (was !== Boolean(input.value().trim())) cx.notify();
      });
      return input;
    });
  }

  entry(address) {
    let e = this.byAgent.get(address);
    if (!e) { e = { card: null, at: 0, busy: false, sending: false, note: null, picks: new Map(), others: new Map() }; this.byAgent.set(address, e); }
    return e;
  }

  card(address) { return this.byAgent.get(address)?.card ?? null; }

  // What the card asks, as a short key: a new request gets a new key (and fades in again).
  key(address) {
    const c = this.card(address);
    return c ? `${c.kind}:${c.command ?? c.title ?? c.questions?.[0]?.question ?? ""}` : "";
  }

  // Reads the card when `want`, at most every CARD_MS unless `force`. Drops it when not wanted.
  async poll(cx, address, want, force = false) {
    if (!address) return;
    const e = this.entry(address);
    if (!want) { if (e.card) { e.card = null; e.note = null; cx.notify(); } return; }
    if (e.busy || e.sending || (!force && Date.now() - e.at < CARD_MS)) return;
    e.busy = true; e.at = Date.now();
    try {
      const r = await sidecar.card(address);
      const line = r.error ? null : r.lines.find((l) => l.type === "card");
      let card = line && line.kind ? line : null;
      // The screen shows one question at a time; keep the full set read with "Show all".
      if (card && e.card && card.request === e.card.request && card.unread > 0 && !e.card.unread) card = e.card;
      // Another view read it in full already.
      const full = shared(KEY_FULL);
      if (card?.unread > 0 && full?.address === address && full.card?.request === card.request) card = full.card;
      const fresh = card && card.request !== e.card?.request;
      if (JSON.stringify(card) !== JSON.stringify(e.card)) {
        if (card?.request !== e.card?.request) { e.picks = new Map(); e.others = new Map(); e.note = null; e.step = 0; if (this.address === address) this.others.forEach((i) => i.set_value("")); }
        e.card = card; cx.notify();
      }
      // A new card takes the keyboard (1-9, Enter), unless you are typing a reply.
      if (fresh && address === this.shown && this.focus && this.canFocus()) this.focus.focus();
      if (!e.read) { e.read = true; cx.notify(); }
    } finally { e.busy = false; }
  }

  // "Show all N questions": walks the picker's tabs (keys go out, on this click only) and keeps the
  // full card, shared so the other views show it too.
  async loadAll(cx, address) {
    const e = this.entry(address);
    if (e.sending || !e.card?.unread) return;
    e.sending = true; e.note = null; cx.notify();
    try {
      const r = await sidecar.card(address, true);
      const line = r.error ? null : r.lines.find((l) => l.type === "card");
      if (line?.kind === "question" && line.request === e.card.request && !line.unread) { e.card = line; setShared(KEY_FULL, { address, card: line }); }
      else e.note = r.error?.message || "Could not read the other questions. Answer them in the agent's terminal.";
    } finally { e.sending = false; cx.notify(); }
  }

  // 1-9 picks an option of the first question without a pick (a multi-choice one: toggles);
  // on an approval 1 allows once and 2 denies. Enter sends the answer. True when it took the key.
  key_down(e, cx, address) {
    const en = this.byAgent.get(address), c = en?.card;
    if (!c || en.sending) return false;
    const n = digit(e);
    if (c.kind === "approval") {
      // Answered already (the sidecar keeps one answer per prompt): the keys stay with whoever has the focus.
      if (c.answered) return false;
      if (n === 1 || n === 2) { cx.spawn((cx) => this.approve(cx, address, n === 1)); return true; }
      return false;
    }
    const qs = c.question?.questions ?? [];
    if (e?.key === "enter") {
      if (c.unread > 0) return false;
      const step = en.step ?? 0;
      if (step < qs.length - 1) {
        if (!answered(en, step)) return false;
        en.step = step + 1; cx.notify(); return true;
      }
      if (answersFor(c.question, en.picks, en.others)) { cx.spawn((cx) => this.answer(cx, address)); return true; }
      return false;
    }
    if (n === null) return false;
    const qi = Math.min(en.step ?? 0, qs.length - 1);
    // The number after the options moves the keyboard into the own-answer box.
    if (n - 1 === (qs[qi]?.options ?? []).length && this.others[qi]) { this.others[qi].focus?.(); return true; }
    if (!this.pick(en, qs, qi, n - 1)) return false;
    cx.notify();
    return true;
  }

  // Picks option `oi` of question `qi`. A one-choice pick moves on to the next question.
  pick(en, qs, qi, oi) {
    const q = qs[qi];
    if (!q || oi >= (q.options ?? []).length) return false;
    const s = new Set(q.multi ? en.picks.get(qi) ?? [] : []);
    if (s.has(oi)) s.delete(oi); else s.add(oi);
    en.picks.set(qi, s);
    if (!q.multi && s.size && qi < qs.length - 1) en.step = qi + 1;
    return true;
  }

  async act(cx, address, run) {
    const e = this.entry(address);
    if (e.sending || !e.card) return;
    e.sending = true; e.note = null; cx.notify();
    let note = null;
    try {
      const r = await run(e.card);
      const done = r.lines.find((l) => l.type === "done");
      if (r.error) note = r.error.message;
      else if (done?.result === "sent") e.card = null;
      else if (sidecar.agentChanged(done)) note = sidecar.AGENT_CHANGED;
      else if (done?.result === "changed") note = "It changed on screen. Here is what it shows now.";
      else note = done?.message || "The keys did not go out.";
    } finally { e.sending = false; }
    await this.poll(cx, address, true, true);
    e.note = note;  // after the read: a new card would clear it
    cx.notify();
  }

  approve(cx, address, allow) { return this.act(cx, address, (c) => sidecar.approve(address, c.request, allow)); }

  answer(cx, address) {
    const e = this.entry(address);
    const answers = answersFor(e.card?.question, e.picks, e.others);
    if (!answers) return;
    return this.act(cx, address, (c) => sidecar.answer(address, c.request, answers));
  }

  // The card for `address`, or null. `id` makes the ids unique when two views show one.
  view(address, t, id = "card") {
    const e = this.byAgent.get(address);
    const c = e?.card;
    if (!c) return null;
    const button = (bid, label, primary, action, off = false) => div().id(`${id}-${bid}`).h(30).px(14).flex().items_center().rounded(8).text_size(13)
      .bg(off ? t.chrome : primary ? t.accent : t.chrome).text_color(off ? t.dim : primary ? t.background : t.foreground)
      .border(1).border_color(primary && !off ? t.accent : t.border)
      .when(!off, (el) => el.hover((s) => s.opacity(0.9)).on_click((_e, cx) => cx.spawn((cx) => action(cx))))
      .child(label);
    const box = v_flex().id(id).w_full().max_w(800).p(14).gap(10).rounded(14).bg(t.raised);
    if (this.focus) box.track_focus(this.focus)
      .on_mouse_down("left", () => this.focus.focus())
      .on_key_down((ev, cx) => { if (this.key_down(ev, cx, address)) cx.stop_propagation(); });
    const hint = (n) => div().size(18).flex_none().rounded(5).border(1).border_color(t.border).flex().items_center().justify_center()
      .text_size(10.5).text_color(t.muted).child(String(n));
    const head = (text) => h_flex().gap(8).items_center()
      .child(div().size(8).flex_none().rounded(4).bg(t.blocked))
      .child(div().text_size(13.5).font_semibold().text_color(t.foreground).child(text));
    const tail = () => e.note ? div().id(`${id}-note`).text_size(12).text_color(t.blocked).child(e.note) : null;

    if (c.kind === "approval") {
      box.child(head(c.title || (c.tool ? `${c.tool} wants to run` : "Approval needed")));
      if (c.command) box.child(div().id(`${id}-command`).max_h(140).overflow_y_scrollbar().px(10).py(8).rounded(8).bg(t.background)
        .text_size(12.5).font_family("Menlo").text_color(t.foreground).child(c.command));
      // Answered and still drawn (the agent has not moved on, or hc still holds the key): no second answer.
      const off = e.sending || c.answered === true;
      box.child(h_flex().gap(8).items_center()
        .child(button("allow", e.sending ? "Sending…" : "1  Allow once", true, (cx) => this.approve(cx, address, true), off))
        .child(button("deny", "2  Deny", false, (cx) => this.approve(cx, address, false), off))
        .when(c.answered === true, (el) => el.child(div().text_size(12).text_color(t.muted).child("Answer sent. Waiting for the agent."))));
      const n = tail(); if (n) box.child(n);
      return box;
    }

    const q = c.question ?? { questions: [] };
    // One question at a time, as in Flockbook: "Question 1 of 2", Back and Next, Send on the last.
    const count = q.questions.length, step = Math.min(e.step ?? 0, Math.max(0, count - 1));
    box.child(h_flex().gap(10).items_center()
      .child(head(count > 1 ? `The agent asks · question ${step + 1} of ${count}` : "The agent asks"))
      .when(count > 1, (el) => el.child(div().flex_1()).child(h_flex().gap(5).children(q.questions.map((_x, i) =>
        div().id(`${id}-dot-${i}`).size(7).rounded(4).bg(i === step ? t.accent : answered(e, i) ? t.muted : t.border)
          .on_click((_ev, cx) => { e.step = i; cx.notify(); }))))));
    q.questions.forEach((qq, qi) => {
      if (qi !== step) return;
      const picks = e.picks.get(qi) ?? new Set();
      const sec = v_flex().gap(6);
      if (qq.header) sec.child(div().text_size(11.5).text_color(t.muted).child(qq.header));
      sec.child(div().text_size(13.5).text_color(t.foreground).child(qq.text || ""));
      (qq.options ?? []).forEach((o, oi) => {
        const on = picks.has(oi);
        sec.child(h_flex().id(`${id}-q${qi}-o${oi}`).px(10).py(7).gap(10).items_center().rounded(8)
          .bg(on ? t.selected : t.background).border(1).border_color(on ? t.accent : t.border).hover((s) => s.bg(on ? t.selected : t.hover))
          .on_click((_e, cx) => { this.pick(e, q.questions, qi, oi); cx.notify(); })
          .child(div().size(14).flex_none().rounded(qq.multi ? 3 : 7).border(1).border_color(on ? t.accent : t.muted).bg(on ? t.accent : t.background))
          .child(v_flex().flex_1().min_w(0)
            .child(h_flex().gap(6).child(div().text_size(13).text_color(t.foreground).child(o.label || ""))
              .when(Boolean(o.recommended), (el) => el.child(div().text_size(11).text_color(t.accent).child("Recommended"))))
            .when(Boolean(o.detail), (el) => el.child(div().text_size(12).text_color(t.muted).child(o.detail))))
          .when(oi < 9, (el) => el.child(hint(oi + 1))));
      });
      // The boxes are shared by every card; fill them with this card's text when it is shown.
      const input = this.others[qi];
      if (this.address !== address) { this.others.forEach((b, k) => b.set_value(e.others.get(k) ?? "")); this.address = address; }
      // Own answer: a row like the options. Clicks and keys stay in the text box: the card would
      // take the focus on a press and read 1-9 and Enter as picks.
      const own = Boolean((e.others.get(qi) ?? "").trim());
      const keep = (_ev, cx) => cx.stop_propagation();
      if (input) sec.child(h_flex().id(`${id}-q${qi}-other`).px(10).py(7).gap(10).items_center().rounded(8)
        .bg(own ? t.selected : t.background).border(1).border_color(own ? t.accent : t.border)
        .on_mouse_down("left", keep).on_key_down(keep)
        .child(div().size(14).flex_none().rounded(qq.multi ? 3 : 7).border(1).border_color(own ? t.accent : t.muted).bg(own ? t.accent : t.background))
        .child(div().flex_1().min_w(0).child(Textarea.new(input).w_full().text_size(13)))
        .when((qq.options ?? []).length < 9, (el) => el.child(hint((qq.options ?? []).length + 1))));
      box.child(sec);
    });
    if (c.unread > 0) {
      const total = q.questions.length + c.unread;
      box.child(h_flex().gap(10).items_center()
        .child(button("all", e.sending ? "Reading…" : `Show all ${total} questions`, true, (cx) => this.loadAll(cx, address), e.sending))
        .child(div().text_size(12.5).text_color(t.muted)
          .child(`${c.unread} more question${c.unread === 1 ? " follows" : "s follow"}. Answer them all here, then send.`)));
    } else {
      const ready = Boolean(answersFor(q, e.picks, e.others));
      const last = step >= count - 1;
      // The hint on the left; Back, then Next or Send answer at the right end.
      box.child(h_flex().gap(10).items_center()
        .child(div().text_size(12).text_color(t.dim).child(count ? `1-9 picks · Enter ${last ? "sends" : "goes on"}` : ""))
        .child(div().flex_1())
        .when(step > 0, (el) => el.child(button("back", "Back", false, async (cx) => { e.step = step - 1; cx.notify(); })))
        .child(last
          ? button("answer", e.sending ? "Sending…" : "Send answer", true, (cx) => this.answer(cx, address), !ready || e.sending)
          : button("next", "Next", true, async (cx) => { e.step = step + 1; cx.notify(); }, !answered(e, step))));
    }
    const n = tail(); if (n) box.child(n);
    return box;
  }
}
