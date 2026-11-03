// What an agent waits for: an approval ("Do you want to run …?") or a question, as a card with
// buttons. Read with the sidecar's `card`; answered with `approve` and `answer` (docs/protocol.md).
// The keys go out only on a button press, never on a timer.
import { div, svg } from "gpui-kit";
import { v_flex, h_flex, Textarea, TextareaState } from "gpui-base";
import { icon } from "./state.js";
import * as sidecar from "./sidecar.js";

export const CARD_MS = 1500;

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

export class Cards {
  // The "other answer" boxes are made here, once: a view may not make input state while it draws.
  constructor() {
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
      const card = line && line.kind ? line : null;
      if (JSON.stringify(card) !== JSON.stringify(e.card)) {
        if (card?.request !== e.card?.request) { e.picks = new Map(); e.others = new Map(); e.note = null; if (this.address === address) this.others.forEach((i) => i.set_value("")); }
        e.card = card; cx.notify();
      }
      if (!e.read) { e.read = true; cx.notify(); }
    } finally { e.busy = false; }
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
    const box = v_flex().id(id).w_full().max_w(800).p(14).gap(10).rounded(14).bg(t.raised).border(1).border_color(t.border).shadow_sm();
    const head = (text) => h_flex().gap(8).items_center()
      .child(div().size(8).flex_none().rounded(4).bg(t.blocked))
      .child(div().text_size(13.5).font_semibold().text_color(t.foreground).child(text));
    const tail = () => e.note ? div().id(`${id}-note`).text_size(12).text_color(t.blocked).child(e.note) : null;

    if (c.kind === "approval") {
      box.child(head(c.title || (c.tool ? `${c.tool} wants to run` : "Approval needed")));
      if (c.command) box.child(div().id(`${id}-command`).max_h(140).overflow_y_scrollbar().px(10).py(8).rounded(8).bg(t.background)
        .text_size(12.5).font_family("Menlo").text_color(t.foreground).child(c.command));
      box.child(h_flex().gap(8)
        .child(button("allow", e.sending ? "Sending…" : "Allow once", true, (cx) => this.approve(cx, address, true), e.sending))
        .child(button("deny", "Deny", false, (cx) => this.approve(cx, address, false), e.sending)));
      const n = tail(); if (n) box.child(n);
      return box;
    }

    const q = c.question ?? { questions: [] };
    box.child(head(q.questions.length > 1 ? `The agent asks ${q.questions.length} questions` : "The agent asks"));
    q.questions.forEach((qq, qi) => {
      const picks = e.picks.get(qi) ?? new Set();
      const sec = v_flex().gap(6);
      if (qq.header) sec.child(div().text_size(11.5).text_color(t.muted).child(qq.header));
      sec.child(div().text_size(13.5).text_color(t.foreground).child(qq.text || ""));
      (qq.options ?? []).forEach((o, oi) => {
        const on = picks.has(oi);
        sec.child(h_flex().id(`${id}-q${qi}-o${oi}`).px(10).py(7).gap(10).items_center().rounded(8)
          .bg(on ? t.selected : t.background).border(1).border_color(on ? t.accent : t.border).hover((s) => s.bg(on ? t.selected : t.hover))
          .on_click((_e, cx) => {
            const s = new Set(qq.multi ? picks : []);
            if (s.has(oi)) s.delete(oi); else s.add(oi);
            e.picks.set(qi, s); cx.notify();
          })
          .child(div().size(14).flex_none().rounded(qq.multi ? 3 : 7).border(1).border_color(on ? t.accent : t.muted).bg(on ? t.accent : t.background))
          .child(v_flex().flex_1().min_w(0)
            .child(h_flex().gap(6).child(div().text_size(13).text_color(t.foreground).child(o.label || ""))
              .when(Boolean(o.recommended), (el) => el.child(div().text_size(11).text_color(t.accent).child("Recommended"))))
            .when(Boolean(o.detail), (el) => el.child(div().text_size(12).text_color(t.muted).child(o.detail)))));
      });
      // The boxes are shared by every card; fill them with this card's text when it is shown.
      const input = this.others[qi];
      if (this.address !== address) { this.others.forEach((b, k) => b.set_value(e.others.get(k) ?? "")); this.address = address; }
      if (input) sec.child(div().id(`${id}-q${qi}-other`).px(8).py(4).rounded(8).bg(t.background).border(1).border_color(t.border).child(Textarea.new(input).w_full()));
      box.child(sec);
    });
    if (c.unread > 0) {
      box.child(h_flex().gap(6).items_center().text_size(12.5).text_color(t.muted)
        .child(svg(icon("bell")).size(12))
        .child(`${c.unread} more question${c.unread === 1 ? "" : "s"} follow. Answer them in the agent's terminal.`));
    } else {
      const ready = Boolean(answersFor(q, e.picks, e.others));
      box.child(h_flex().gap(8).child(button("answer", e.sending ? "Sending…" : "Send answer", true, (cx) => this.answer(cx, address), !ready || e.sending)));
    }
    const n = tail(); if (n) box.child(n);
    return box;
  }
}
