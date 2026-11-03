// The "top" view: the sidebar button, a capsule with one tab per tab of the shown space, a capsule
// with "+" and the terminal button.
import { div, svg, image } from "gpui-kit";
import { h_flex } from "gpui-base";
import { theme, layout, window, settings, openOverlay, run, setAgentOrder, nextCommand, shared, setShared } from "herdr";
import { openMenu, tabItems } from "./menu.js";
import { spaceAgents, spaceTabs, activeTab, currentAgent, tabLabel, screenFor, showTab, closeTab, openTerminal, showNeeds, icon, tabAgents, shownScreen,
  SPIN_MS, FADE, sidebarShown, toggleSidebar, loadSidebar, showScreen, showOverview, watchEdge } from "./state.js";

function find(node, id) {
  if (node?.id === id) return node;
  for (const c of node?.children ?? []) { const f = find(c, id); if (f) return f; }
  return null;
}

// The status mark at the start of a tab: a spinner while working, a check when done, a dot when it
// waits for you, an empty ring otherwise (idle, or no agent).
export function statusMark(status, t) {
  if (status === "working") return svg(icon("spinner")).size(13).flex_none().text_color(t.working).spin(SPIN_MS);
  if (status === "done") return svg(icon("check")).size(13).flex_none().text_color(t.done);
  if (status === "blocked") return div().size(9).m(2).flex_none().rounded(5).bg(t.blocked);
  return div().size(12).m(0.5).flex_none().rounded(6).border(1.3).border_color(t.dim);
}

const SYNC_MS = 300;

// Room left of the bar for the macOS window buttons (no title bar), else a plain margin.
export function lightsRoom() {
  const root = layout(), w = window();
  return w?.platform === "macos" && w.titlebar === "none" && !w.fullscreen && root?.traffic_lights
    ? (root.traffic_lights[0] + 60) / (settings().zoom ?? 1) : 12;
}

export const BAR_H = 56;
const KEY_CMD = "flockbook.command";   // the last key command and when, so two copies don't both act

// Keys bound to "flockbook.<command>" in keys.json.
export function command(c) {
  const tabs = spaceTabs(), tab = activeTab(), i = tabs.findIndex((x) => x.id === tab?.id);
  if (c === "terminal") return openTerminal(currentAgent());
  if (c === "needs") return showNeeds();
  if (c === "sidebar") return toggleSidebar();
  if (c === "overview") return showOverview();
  if ((c === "move-left" || c === "move-right") && i >= 0) {
    const to = i + (c === "move-left" ? -1 : 1);
    if (to >= 0 && to < tabs.length) run("tab.move", { id: tab.id, index: tabs[to].index });
  }
}

// Each screen (chat, Needs you, Overview, the tab's terminals) draws the bar, so each runs its
// work: the key commands, the screen sync, the saved sidebar state. Should the app hand one key
// to every copy, the first acts and the others see it in KEY_CMD and skip it.
export function startTop(cx, owner) {
  owner.seen = undefined;
  cx?.timer?.every(SYNC_MS, () => syncScreen(owner));
  cx?.spawn?.(async (cx) => {
    for (;;) {
      const c = await nextCommand();
      const last = shared(KEY_CMD), at = Date.now();
      if (last && last.c === c && at - last.at < 150) continue;
      setShared(KEY_CMD, { c, at });
      command(c); cx.notify();
    }
  });
  cx?.spawn?.(async (cx) => { await loadSidebar(); cx.notify(); });
}

// Keeps the screen right for the shown tab: the chat for an agent, the tab's terminals for a
// shell. It acts only when the tab changes or an agent starts or stops in it, so a choice
// made by hand (Needs you, the tab's terminals) stays until then.
export function syncScreen(owner) {
  const tab = activeTab();
  const now = tab ? `${tab.id}:${screenFor(tab)}` : null;
  if (now === owner.seen) return;
  owner.seen = now;
  if (!tab) return;
  // layout() is the file, not the live tree, so the shown screen is unknown: select it outright.
  showScreen(screenFor(tab));
}

// The bar: the sidebar button, the tab capsule, the tools capsule. `over`: drawn over a screen's
// content, so only the capsules and buttons have a background and the rest shows what scrolls
// under it.
export function topBar(t, over = false) {
  setAgentOrder(spaceAgents().map((a) => a.address));
  const tabs = spaceTabs();
  const cur = currentAgent();
  const side = sidebarShown();
  // While the sidebar is hidden, the sidebar button sits right of the window buttons; while it
  // shows, the button is in the sidebar (side.js) and the bar starts at its edge.
  const left = side ? 12 : lightsRoom();
  const stop = (_e, cx) => cx.stop_propagation();
  const button = (id, child, action, size = 28, tip = null) => div().id(id).when(Boolean(tip), (el) => el.tooltip(tip)).size(size).flex_none().flex().items_center().justify_center().rounded(size / 2)
    .text_color(t.muted).hover((s) => s.bg(t.hover).text_color(t.foreground))
    .on_mouse_down("left", stop).on_click(action).child(child);
  const capsule = (id) => h_flex().id(id).h(38).px(4).gap(2).items_center().flex_none().rounded(19)
    .bg(t.chrome).border(1).border_color(t.border).shadow_sm();

  const pill = (tab) => {
    const on = tab.active;
    return h_flex().id(`tab-${tab.id}`).relative().h(30).pl(13).pr(on ? 6 : 14).gap(8).items_center().flex_none().rounded(15).text_size(13)
      .text_color(on ? t.foreground : t.muted)
      // The selected fill fades in and out instead of jumping between tabs.
      .child(div().id(`fill-${tab.id}`).absolute().inset_0().rounded(15).bg(t.selected).opacity(on ? 1 : 0).transition("opacity", FADE))
      .when(on, (el) => el.font_weight(600))
      .when(!on, (el) => el.hover((s) => s.bg(t.hover).text_color(t.foreground)))
      .on_mouse_down("left", stop).on_click(() => showTab(tab))
      .on_mouse_down("right", (_e, cx) => { cx.stop_propagation(); openMenu(tabItems(tab, { agent: tabAgents(tab.id)[0] ?? null, list: tabs })); })
      .child(statusMark(tab.status, t))
      .child(div().max_w(170).truncate().child(tabLabel(tab)))
      .when(on, (el) => el.child(div().id(`close-${tab.id}`).size(18).flex_none().flex().items_center().justify_center().rounded(9)
        .text_size(15).text_color(t.muted).hover((s) => s.bg(t.hover).text_color(t.foreground))
        .on_mouse_down("left", stop)
        .on_click((_e, cx) => { cx.stop_propagation(); closeTab(tab.id); }).child("×")));
  };

  const strip = capsule("tabs").min_w(0).flex_shrink(1)
    .child(tabs.length ? h_flex().id("tab-scroll").min_w(0).gap(4).items_center().overflow_x_scrollbar().children(tabs.map(pill))
      : div().px(12).text_size(12.5).text_color(t.muted).child("No tabs in this space"));
  const tools = capsule("tools").px(5).gap(4)
    .child(button("new", svg(icon("plus")).size(15), () => openOverlay({ mod: "new-menu", kind: "popup", width: 250, height: 512 }), 30, "New tab (⌘T)"))
    .when(Boolean(cur), (el) => el.child(button("terminal", svg(icon("terminal")).size(16), () => openTerminal(cur), 30, "Agent's terminal (⌃`)")))
    // Chat ⇄ the tab's panes (splits show there; ⌘D splits).
    .when(Boolean(cur), (el) => el.child(button("panes", svg(icon(shownScreen() === "tab" ? "send" : "split")).size(15),
      () => showScreen(shownScreen() === "tab" ? "chat" : "tab"), 30, shownScreen() === "tab" ? "Show the chat" : "Show the tab's panes (splits)")));

  // The right padding matches the left, so the capsules stay centred. The sidebar button gets a
  // round backing when drawn over content, like the capsules.
  return watchEdge(h_flex().id("top-root").w_full().h(BAR_H).min_w(0)).pl(left).pr(left + (side ? 0 : 36)).gap(8).items_center()
    .when(!over, (el) => el.bg(t.background))
    .when(!side, (el) => el.child(button("sidebar", svg(icon("sidebar")).size(16), () => toggleSidebar(), 28, "Sidebar (⌘B)")
      .when(over, (b) => b.bg(t.chrome).border(1).border_color(t.border))))
    .child(h_flex().flex_1().min_w(0).justify_center().gap(10).child(strip).child(tools));
}

// Puts the bar over a screen's content (absolute, at the top).
export const overBar = (t) => div().id("over-bar").absolute().top(0).left(0).right(0).h(BAR_H).child(topBar(t, true));

export class TopView {
  constructor(cx) { startTop(cx, this); }
  sync() { syncScreen(this); }
  command(c) { command(c); }
  render(_cx) { return topBar(theme()); }
}
