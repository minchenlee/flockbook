// The "needs" view: the agents that wait for the owner on the left, the one picked on the right.
// The right side shows what the picked agent waits for (an approval or a question card) and opens its chat.
import { div, svg, image } from "gpui-kit";
import { v_flex, h_flex } from "gpui-base";
import { theme } from "herdr";
import { needing, agentName, showAgent, icon, watchEdge } from "./state.js";
import { Cards } from "./cards.js";
import { startTop, overBar, BAR_H } from "./top.js";

export class NeedsView {
  constructor(cx) {
    this.picked = null;
    this.cards = new Cards(cx);
    this.top = {}; startTop(cx, this.top);
    this.shown = null;   // the agent on the right, from the last render
    cx.timer.every(500, (cx) => this.tick(cx));
  }

  tick(cx) {
    const list = needing();
    const cur = list.find((a) => a.address === this.picked) ?? list[0] ?? null;
    this.shown = cur?.address ?? null;
    if (cur) cx.spawn((cx) => this.cards.poll(cx, cur.address, true));
  }

  // The screen with the bar drawn over it. Presses on the content don't drag the window.
  render(cx) {
    const t = theme();
    const stop = (_e, cx) => cx.stop_propagation();
    return v_flex().id("needs-screen").relative().size_full().bg(t.background)
      .child(div().size_full().pt(BAR_H).on_mouse_down("left", stop).child(this.content(t)))
      .child(overBar(t));
  }

  content(t) {
    const list = needing();
    if (!list.length) {
      return v_flex().size_full().items_center().justify_center().gap(8).text_color(t.muted)
        .child(svg(icon("check")).size(28).text_color(t.done))
        .child(div().text_size(17).font_semibold().text_color(t.foreground).child("All clear"))
        .child(div().text_size(13).child("No agent waits for you."));
    }
    const cur = list.find((a) => a.address === this.picked) ?? list[0];
    const item = (a) => {
      const on = a.address === cur.address;
      const p = a.program;
      return h_flex().id(`need-${a.address}`).h(52).px(12).gap(10).items_center().rounded(8)
        .bg(on ? t.selected : t.background).hover((s) => s.bg(on ? t.selected : t.hover))
        .on_click((_e, cx) => { this.picked = a.address; cx.notify(); })
        .when(Boolean(p?.icon), (el) => el.child(p.tint ? svg(p.icon).size(18).text_color(t.muted) : image(p.icon).size(18)))
        .child(v_flex().flex_1().min_w(0)
          .child(div().truncate().text_size(13.5).text_color(t.foreground).child(agentName(a)))
          .child(div().truncate().text_size(11.5).text_color(t.muted).child(`${a.space_name} · waits for you`)))
        .child(div().size(8).flex_none().rounded(4).bg(t.blocked));
    };
    // Both columns start at the top (h_flex centres its children by default).
    return watchEdge(h_flex().id("needs-root").size_full().min_h(0).items_start())
      .child(v_flex().id("needs-list").w(340).flex_none().h_full().p(10).gap(2).border_r(1).border_color(t.border)
        .overflow_y_scrollbar()
        .child(div().px(4).pb(6).text_size(15).font_semibold().text_color(t.foreground).child(`Needs you · ${list.length}`))
        .children(list.map(item)))
      .child(v_flex().id("needs-detail").flex_1().min_w(0).h_full().overflow_y_scrollbar().p(24).gap(12)
        .child(div().text_size(18).font_semibold().text_color(t.foreground).child(agentName(cur)))
        .child(div().text_size(13).text_color(t.muted).child(cur.title || `${cur.space_name} · ${cur.tab_name}`))
        .child(this.cards.view(cur.address, t, "needs-card") ?? div().text_size(13).text_color(t.muted)
          .child(this.cards.byAgent.get(cur.address)?.read ? "It waits for you in its terminal. Open the chat to see why." : "Reading what it waits for…"))
        .child(div().id("needs-open").self_start().px(14).h(32).flex().items_center().rounded(8).text_size(13)
          .bg(t.accent).text_color(t.background).hover((s) => s.opacity(0.9))
          .on_click(() => showAgent(cur.address)).child("Open chat")));
  }
}
