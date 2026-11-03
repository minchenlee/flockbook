// The "overview" view: every tab of every space as a card, grouped by status or by space.
import { div, svg } from "gpui-kit";
import { v_flex, h_flex } from "gpui-base";
import { theme, spaces, shared, setShared } from "herdr";
import { tabLabel, tabAgents, agentName, openTab, icon, FADE, watchEdge, noteWindowHeight } from "./state.js";
import { openMenu, tabItems } from "./menu.js";
import { statusMark, startTop, overBar, BAR_H } from "./top.js";

export const KEY_GROUP = "flockbook.overview.group";   // "status" | "space"
const CARD_MIN = 300, GAP = 12, MAX_W = 1220;

export function greeting(hour) {
  return hour < 5 ? "Good evening" : hour < 12 ? "Good morning" : hour < 18 ? "Good afternoon" : "Good evening";
}

// "now", "13m", "5h", "2d" since `ms`; "" when unknown.
export function ago(ms, now = Date.now()) {
  if (!ms) return "";
  const m = Math.floor((now - ms) / 60000);
  if (m < 1) return "now";
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  return h < 24 ? `${h}h` : `${Math.floor(h / 24)}d`;
}

// Every tab that is not put away, with its space.
export function allTabs() {
  return spaces().flatMap((s) => (s.tabs ?? []).filter((t) => !t.settled).map((tab) => ({ space: s, tab })));
}

const STATUS_GROUPS = [
  ["Needs you", (st) => st === "blocked"],
  ["Working", (st) => st === "working"],
  ["Done and resting", (st) => st !== "blocked" && st !== "working"],
];

// The sections to show: [{ title, items }], empty ones left out.
export function groups(items, by) {
  if (by === "space") {
    const out = new Map();
    for (const it of items) {
      if (!out.has(it.space.id)) out.set(it.space.id, { title: it.space.name, items: [] });
      out.get(it.space.id).items.push(it);
    }
    return [...out.values()];
  }
  return STATUS_GROUPS.map(([title, test]) => ({ title, items: items.filter((it) => test(it.tab.status)) }))
    .filter((g) => g.items.length);
}

export class OverviewView {
  constructor(cx) {
    this.width = 900;
    this.top = {}; startTop(cx, this.top);
    // The "13m" labels age; redraw twice a minute.
    cx?.timer?.every?.(30000, (cx) => cx.notify());
  }

  card(it, t, w) {
    const { space, tab } = it;
    const a = tabAgents(tab.id)[0];
    const sub = [space.name, a ? agentName(a) : null].filter(Boolean).filter((x, i, l) => l.indexOf(x) === i && x !== tabLabel(tab)).join(" · ");
    const when = ago(tab.status === "working" ? tab.working_since ?? tab.changed_at : tab.changed_at);
    return h_flex().id(`ov-${tab.id}`).w(w).h(64).px(16).gap(12).items_center().flex_none().rounded(12)
      .bg(t.raised).border(1).border_color(t.border).hover((s) => s.bg(t.hover))
      .on_click(() => openTab(space.id, tab))
      .on_mouse_down("right", (_e, cx) => { cx.stop_propagation(); openMenu(tabItems(tab, { agent: a ?? null, spaceId: space.id, list: (space.tabs ?? []).filter((x) => !x.settled) })); })
      .child(h_flex().size(20).flex_none().items_center().justify_center().child(statusMark(tab.status, t)))
      .child(v_flex().flex_1().min_w(0).gap(2)
        .child(h_flex().gap(8).items_center()
          .child(div().flex_1().min_w(0).truncate().text_size(14).font_semibold().text_color(t.foreground).child(tabLabel(tab)))
          .when(Boolean(when), (el) => el.child(div().flex_none().text_size(11.5).text_color(t.muted).child(when))))
        .child(div().truncate().text_size(12).text_color(t.muted).child(sub || " ")));
  }

  segmented(t, by) {
    const opt = (key, label) => div().id(`group-${key}`).h(26).px(12).flex().items_center().rounded(7).text_size(12.5)
      .text_color(by === key ? t.foreground : t.muted).when(by === key, (el) => el.bg(t.selected))
      .when(by !== key, (el) => el.hover((s) => s.text_color(t.foreground)))
      .on_click((_e, cx) => { setShared(KEY_GROUP, key); cx.notify(); }).child(label);
    return h_flex().gap(8).items_center()
      .child(div().text_size(12.5).text_color(t.muted).child("Group by"))
      .child(h_flex().p(2).gap(2).rounded(9).bg(t.chrome).border(1).border_color(t.border)
        .child(opt("status", "Status")).child(opt("space", "Workspace")));
  }

  render(_cx) {
    const t = theme();
    const by = shared(KEY_GROUP) ?? "status";
    const items = allTabs();
    const working = items.filter((it) => it.tab.status === "working").length;
    const waiting = items.filter((it) => it.tab.status === "blocked").length;
    const inner = Math.min(MAX_W, this.width - 64);
    const cols = Math.max(1, Math.min(3, Math.floor((inner + GAP) / (CARD_MIN + GAP))));
    const w = Math.floor((inner - GAP * (cols - 1)) / cols);

    const section = (g) => v_flex().gap(10)
      .child(div().text_size(13.5).font_semibold().text_color(t.foreground).child(g.title))
      .child(h_flex().flex_wrap().gap(GAP).children(g.items.map((it) => this.card(it, t, w))));

    const body = v_flex().w(inner).gap(28).pt(BAR_H + 20).pb(48)
      .child(h_flex().items_start()
        .child(v_flex().flex_1().gap(4)
          .child(div().text_size(26).font_semibold().text_color(t.foreground).child(greeting(new Date().getHours())))
          .child(div().text_size(13.5).text_color(t.muted)
            .child(`${items.length} ${items.length === 1 ? "tab" : "tabs"} · ${working} working · ${waiting} need you`)))
        .child(this.segmented(t, by)))
      .children(items.length ? groups(items, by).map(section)
        : [div().pt(40).text_size(13).text_color(t.muted).child("No tabs yet. Start one with +.")]);

    return watchEdge(v_flex().id("overview").size_full().bg(t.background))
      
      .on_size_change((size, cx) => { noteWindowHeight(size); if (Math.abs(size.width - this.width) >= 1) { this.width = size.width; cx.notify(); } })
      .relative()
      .child(v_flex().id("overview-scroll").size_full().items_center().overflow_y_scrollbar()
        .on_mouse_down("left", (_e, cx) => cx.stop_propagation()).child(body))
      .child(overBar(t));
  }
}
