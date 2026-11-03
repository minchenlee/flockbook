// The "side" view: New tab, Needs you, Overview, the spaces with a status mark per tab, Settings.
import { div, svg } from "gpui-kit";
import { v_flex, h_flex } from "gpui-base";
import { theme, spaces, run, openOverlay, closeOverlay, props } from "herdr";
import { openMenu, spaceItems } from "./menu.js";
import { needing, showNeeds, showOverview, shownScreen, icon, showSpace, toggleSidebar, peekClosed, PEEK_W, SPRING } from "./state.js";
import { statusMark, lightsRoom } from "./top.js";

// Status marks for a space's tabs: all of them up to 5, else the first 3 and "+N".
export function tabMarks(tabs) {
  const live = tabs.filter((t) => t.status);
  if (live.length <= 5) return { shown: live, more: 0 };
  return { shown: live.slice(0, 3), more: live.length - 3 };
}

// Two letters for the monogram: the first letters of the first two words, else the first two letters.
export function monogram(name) {
  const words = String(name || "?").split(/[\s\-_/.]+/).filter(Boolean);
  const s = words.length >= 2 ? words[0][0] + words[1][0] : (words[0] || "?").slice(0, 2);
  return s.toUpperCase();
}

export class SideView {
  // `peek`: this copy is the panel that floats over the content (a popup opened at the left edge).
  constructor(cx) {
    this.peek = props()?.peek === true ? props().overlay ?? null : null;
    this.floating = props()?.peek === true;
    // Slide in: the first frame left of its spot, then a spring to it.
    this.slid = !this.floating;
    if (!this.slid) cx?.timer?.after?.(16, (cx2) => { this.slid = true; cx2?.notify?.(); });
  }

  // The panel goes when the pointer leaves it.
  leave() {
    if (!this.floating) return;
    const id = props()?.overlay ?? this.peek;
    if (id) { closeOverlay(id); peekClosed(id); }
  }

  close(_cx) { const id = props()?.overlay ?? this.peek; if (this.floating && id) peekClosed(id); }

  render(_cx) {
    const t = theme();
    const stop = (_e, cx) => cx.stop_propagation();
    const screen = shownScreen();
    const row = (id, iconName, label, action, extra, on = false) => h_flex().id(id).h(30).px(10).gap(9).items_center().rounded(7)
      .text_size(13).text_color(t.foreground).when(on, (el) => el.bg(t.selected)).hover((s) => s.bg(on ? t.selected : t.hover))
      .on_mouse_down("left", stop).on_click(action)
      .child(h_flex().w(22).flex_none().justify_center().child(svg(icon(iconName)).size(15).text_color(t.muted)))
      .child(div().flex_1().min_w(0).truncate().child(label))
      .when(Boolean(extra), (el) => el.child(extra));

    const waiting = needing().length;
    const badge = waiting ? div().px(6).h(18).rounded(9).flex().items_center().text_size(11).font_semibold()
      .bg(t.blocked).text_color(t.background).child(String(waiting)) : null;

    // Every row: 10 px in, a 22 px icon or monogram column, then the label, so the labels, the
    // "Workspaces" heading and the right edges (badge, marks, +) line up.
    const space = (s) => {
      const { shown, more } = tabMarks(s.tabs ?? []);
      return h_flex().id(`space-${s.id}`).h(34).px(10).gap(9).items_center().rounded(7)
        .bg(s.active ? t.selected : t.chrome).hover((x) => x.bg(s.active ? t.selected : t.hover))
        .on_mouse_down("left", stop).on_click(() => showSpace(s.id))
        .on_mouse_down("right", (_e, cx) => { cx.stop_propagation(); openMenu(spaceItems(s)); })
        .child(div().size(22).flex_none().rounded(6).flex().items_center().justify_center().text_size(10.5).font_semibold()
          .bg(t.raised).border(1).border_color(t.border).text_color(t.muted).child(monogram(s.name)))
        .child(div().flex_1().min_w(0).truncate().text_size(13).text_color(s.active ? t.foreground : t.muted).child(s.name))
        .child(h_flex().gap(4).items_center().flex_none()
          .children(shown.map((tab) => statusMark(tab.status, t) ?? div().size(6).rounded(3).bg(t.dim)))
          .when(more > 0, (el) => el.child(div().text_size(11).text_color(t.muted).child(`+${more}`))));
    };

    // The header: the sidebar button right of the window buttons, where the top bar has it while
    // the sidebar is hidden, so it does not jump when the sidebar opens or closes.
    const header = h_flex().h(56).flex_none().pl(Math.max(0, lightsRoom() - 8)).items_center()
      .child(div().id("side-toggle").size(28).flex_none().flex().items_center().justify_center().rounded(14)
        .text_color(t.muted).hover((s) => s.bg(t.hover).text_color(t.foreground))
        .on_mouse_down("left", stop).on_click(() => toggleSidebar()).child(svg(icon("sidebar")).size(16)));
    const root = v_flex().id("side-root").size_full().px(8).pb(8).gap(2)
      .on_hover((h) => { if (!h) this.leave(); });
    // Floating: the panel draws its own surface, as Herdr draws no frame around it.
    if (this.floating) root.on_mouse_down("left", stop).bg(t.chrome).rounded(14).border(1).border_color(t.border).shadow_lg()
      .relative().left(this.slid ? 0 : -PEEK_W).when(this.slid, (el) => el.spring("left", SPRING));
    return root
      .child(header)
      .child(row("side-new", "plus", "New tab", () => openOverlay({ mod: "new-menu", kind: "popup", width: 250, height: 512 })))
      .child(row("side-needs", "bell", "Needs you", () => showNeeds(), badge, screen === "needs"))
      .child(row("side-overview", "grid", "Overview", () => showOverview(), null, screen === "overview"))
      .child(h_flex().mt(14).mb(2).px(10).items_center()
        .child(div().flex_1().text_size(11.5).font_semibold().text_color(t.muted).child("Workspaces"))
        .child(div().id("space-new").size(20).rounded(5).flex().items_center().justify_center().text_color(t.muted)
          .hover((s) => s.bg(t.hover)).on_mouse_down("left", stop).on_click(() => run("space.new"))
          .child(svg(icon("plus")).size(12))))
      .child(v_flex().id("spaces").flex_1().min_h(0).gap(1).overflow_y_scrollbar().children(spaces().map(space)))
      .child(row("side-settings", "gear", "Settings", () => run("app.settings")));
  }
}
