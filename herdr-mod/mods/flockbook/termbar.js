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
    return h_flex().size_full().px(12).gap(8).items_center().bg(t.chrome).border_b(1).border_color(t.border)
      .child(svg(icon("terminal")).size(13).text_color(t.muted))
      .child(div().flex_1().min_w(0).truncate().text_size(12.5).text_color(t.foreground).child(`${p.name || "Agent"} · terminal`))
      .child(div().id("term-close").h(24).px(10).flex().items_center().rounded(6).text_size(12).text_color(t.muted)
        .border(1).border_color(t.border).hover((s) => s.bg(t.hover).text_color(t.foreground))
        .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
        .on_click(() => { setShared(KEY_TERM, null); closeOverlay(p.overlay); }).child("Close"));
  }
}
