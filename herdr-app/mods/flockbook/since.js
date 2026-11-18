// The chat's "Since you left" card: what came into this conversation while its chat was not on screen.
// Counted from the rows the chat already has and from the helper's `glance` line; nothing is asked of a model.
import { div, svg } from "gpui-kit";
import { v_flex, h_flex } from "gpui-base";
import { icon } from "./state.js";
import { facts } from "./glance.js";

// A shorter absence shows no card.
export const AWAY_MS = 5 * 60000;

// "47 minutes", "3 hours", "2 days".
export function span(ms) {
  const m = Math.max(1, Math.round(ms / 60000));
  if (m < 60) return `${m} ${m === 1 ? "minute" : "minutes"}`;
  const h = Math.round(m / 60);
  if (h < 24) return `${h} ${h === 1 ? "hour" : "hours"}`;
  const d = Math.round(h / 24);
  return `${d} ${d === 1 ? "day" : "days"}`;
}

// Words in a text: runs of letters between spaces, and each CJK character as one.
export function words(text) {
  const s = String(text ?? "");
  const cjk = s.match(/[぀-ヿ㐀-鿿가-힯]/g)?.length ?? 0;
  const rest = s.replace(/[぀-ヿ㐀-鿿가-힯]/g, " ").split(/\s+/).filter((w) => /[\p{L}\p{N}]/u.test(w)).length;
  return cjk + rest;
}

// "about 1,900": two digits kept, so it does not read as an exact count.
export function about(n) {
  if (n < 100) return String(Math.round(n / 10) * 10 || n);
  const p = 10 ** (String(Math.round(n)).length - 2);
  return (Math.round(n / p) * p).toLocaleString("en-US");
}

// "14:02" on this computer's clock.
export function clock(ms) {
  const d = new Date(ms);
  return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}

// The owner came back to this chat. `seen`: the mark from before (glance.js seenFor), `g`: the agent's
// glance line now, `now`: ms. Null when the chat was on screen less than AWAY_MS ago, or nothing came.
export function arrive(address, seen, g, now) {
  if (!seen?.at || !g?.id || seen.id === g.id || now - seen.at < AWAY_MS) return null;
  return { address, from: seen.at, back: now };
}

// What the card says. `rows`: the chat's rows; `away`: from `arrive`.
export function sinceLeft(rows, away, g) {
  const came = (rows ?? []).filter((r) => r.k === "assistant" && r.ts > away.from && r.ts <= away.back);
  const own = (rows ?? []).filter((r) => r.k === "user" && r.from == null);
  const said = own[own.length - 1] ?? null;
  const first = said ? String(said.text ?? "").split("\n").map((l) => l.trim()).find(Boolean) ?? "" : "";
  return {
    away: span(away.back - away.from),
    messages: came.length,
    words: came.reduce((n, r) => n + words(r.text), 0),
    facts: facts(g),
    said: said && first ? { id: said.id, at: said.ts ? clock(said.ts) : "", text: first } : null,
  };
}

// The line of counts: "You were away 47 minutes · 6 new messages · about 1,900 words".
export function countLine(s) {
  const out = [`You were away ${s.away}`];
  if (s.messages) out.push(`${s.messages} new ${s.messages === 1 ? "message" : "messages"}`, `about ${about(s.words)} words`);
  else out.push("no new message in the rows loaded");
  return out.join(" · ");
}

// The card. `summary`: { state, needs } from a model, when Summaries is on and one came (else null).
// `onJump(id)`: scroll the chat to the owner's message. `onHide`: take the card away.
export function sinceCard(t, s, summary, onJump, onHide) {
  const small = (text, color = t.muted) => div().text_size(12.5).text_color(color).child(text);
  return v_flex().id("since-card").w_full().max_w(760).p(14).gap(8).rounded(12).bg(t.raised).border(1).border_color(t.border)
    .child(h_flex().gap(8).items_center()
      .child(div().flex_1().min_w(0).text_size(13.5).font_semibold().text_color(t.foreground).child("Since you left"))
      .child(div().id("since-hide").flex_none().size(22).flex().items_center().justify_center().rounded(6).text_color(t.muted)
        .hover((st) => st.bg(t.hover)).on_click((_e, cx) => onHide(cx)).child(svg(icon("close")).size(11))))
    .child(small(countLine(s)))
    .when(Boolean(summary?.state), (el) => el.child(v_flex().gap(4)
      .child(div().text_size(11.5).text_color(t.dim).child("Summary · written by a model, may be wrong"))
      .child(div().text_size(13).text_color(t.foreground).child(summary.state))
      .when(Boolean(summary.needs), (el) => el.child(div().text_size(13).text_color(t.foreground).child(`Needs you: ${summary.needs}`)))))
    .when(Boolean(s.facts), (el) => el.child(small(`${summary?.state ? "From the log, not from the summary" : "From the log"}: ${s.facts}`)))
    .when(Boolean(s.said), (el) => el.child(h_flex().gap(10).items_center()
      .child(div().flex_1().min_w(0).truncate().text_size(12.5).text_color(t.muted).child(`You said${s.said.at ? ` at ${s.said.at}` : ""}: ${s.said.text}`))
      .child(div().id("since-jump").flex_none().px(10).h(26).flex().items_center().rounded(8).text_size(12)
        .border(1).border_color(t.border).text_color(t.foreground).hover((st) => st.bg(t.hover))
        .on_click((_e, cx) => onJump(s.said.id, cx)).child("Jump to that message"))));
}
