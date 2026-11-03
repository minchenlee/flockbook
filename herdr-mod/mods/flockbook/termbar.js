// The title bar of the terminal window (state.openTerminal): the agent's name and Close.
// The bar drags the window; Esc goes to the terminal, so Close is the way out.
import { div, svg } from "gpui-kit";
import { h_flex } from "gpui-base";
import { theme, props, closeOverlay, setShared } from "herdr";
import { icon, KEY_TERM } from "./state.js";

export class TermBar {
  constructor(_cx) {}

  render(_cx) {
    const t = theme();
    const p = props() ?? {};
    // A title bar: the agent's name with a terminal chip on the left, the close key and a round
    // close button on the right. Presses elsewhere drag the window.
    return h_flex().size_full().pl(10).pr(8).gap(10).items_center().bg(t.chrome).border_b(1).border_color(t.border)
      .child(div().size(24).flex_none().rounded(7).flex().items_center().justify_center().bg(t.raised)
        .border(1).border_color(t.border).child(svg(icon("terminal")).size(13).text_color(t.muted)))
      .child(h_flex().flex_1().min_w(0).gap(8).items_center()
        .child(div().min_w(0).truncate().text_size(13).font_semibold().text_color(t.foreground).child(p.name || "Agent"))
        .child(div().flex_none().text_size(12).text_color(t.muted).child("Terminal")))
      .child(div().flex_none().text_size(11.5).text_color(t.dim).child("⌃` closes"))
      .child(div().id("term-close").size(26).flex_none().flex().items_center().justify_center().rounded(13)
        .text_color(t.muted).hover((s) => s.bg(t.hover).text_color(t.foreground)).tooltip("Close (⌃`)")
        .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
        .on_click(() => { setShared(KEY_TERM, null); closeOverlay(p.overlay); }).child(svg(icon("close")).size(14)));
  }
}
