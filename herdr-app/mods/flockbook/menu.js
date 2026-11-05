// Right-click menus: a popup at the pointer with one row per item. The items are data (a command
// and its arguments, or one of the mod's own actions) because the popup is another copy of the mod.
import { div } from "gpui-kit";
import { v_flex, h_flex } from "gpui-base";
import { theme, props, run, openOverlay, closeOverlay, agents, shared, setShared } from "herdr";
import { openTerminal, showScreen, shownScreen, showAgent, openTab, showSettings, KEY_SCREEN, KEY_REMOVE, machineOf } from "./state.js";
import { setting } from "./config.js";

const ROW_H = 28, PAD = 6, WIDTH = 220;

// Item: { label, cmd?, args?, act?, danger?, sep? }. `sep`: a line above it.
// `at`: where to open; "pointer" by default. A view in an overlay window passes the press's own
// position, as Herdr's pointer is not updated while the pointer is over an overlay.
export function openMenu(items, at = "pointer") {
  const list = items.filter(Boolean);
  if (!list.length) return null;
  const seps = list.filter((i) => i.sep).length;
  return openOverlay({ mod: "flockbook", props: { view: "menu", items: list }, kind: "popup", at,
    width: WIDTH, height: PAD * 2 + list.length * ROW_H + seps * 9 + 2 });
}

// The mod's own actions, by name.
export function act(item) {
  if (item.cmd) return run(item.cmd, item.args ?? {});
  const a = item.args ?? {};
  switch (item.act) {
    case "terminal": return openTerminal(agents().find((x) => x.address === a.address) ?? null);
    case "panes": return showScreen("tab");
    case "chat": return showScreen("chat");
    case "open-agent": return showAgent(a.address);
    case "open-tab": return openTab(a.space, { id: a.tab });
    // The chat shows a card that asks before anything is removed.
    case "remove-helper": setShared(KEY_REMOVE, a.machine); return showScreen("chat");
    case "settings": return showSettings();
    case "copy": return null;   // handled in the view (needs cx)
  }
  return null;
}

// The items for a tab pill or an Overview card.
export function tabItems(tab, { agent = null, spaceId = null, list = [] } = {}) {
  const i = list.findIndex((x) => x.id === tab.id);
  const machine = machineOf(agent?.address);
  const shown = tab.active && !spaceId;   // the bar's shown tab: the screen and split items apply
  return [
    spaceId && !tab.active ? { label: "Open", act: "open-tab", args: { space: spaceId, tab: tab.id } } : null,
    { label: "Rename…", cmd: "tab.rename", args: { id: tab.id } },
    i > 0 ? { label: "Move left", cmd: "tab.move", args: { id: tab.id, index: list[i - 1].index } } : null,
    i >= 0 && i < list.length - 1 ? { label: "Move right", cmd: "tab.move", args: { id: tab.id, index: list[i + 1].index } } : null,
    agent ? { label: "Open terminal", act: "terminal", args: { address: agent.address }, sep: true } : null,
    shown && agent ? (shownScreen() === "tab" ? { label: "Show chat", act: "chat" } : { label: "Show panes", act: "panes" }) : null,
    shown ? { label: "Split right", cmd: "pane.split", args: { direction: "right" } } : null,
    shown ? { label: "Split down", cmd: "pane.split", args: { direction: "down" } } : null,
    // Never: nothing runs there, and nothing is removed from here either; the line says where to change that.
    machine ? (setting("remote_helper") !== "never"
      ? { label: `Remove helper from ${machine}…`, act: "remove-helper", args: { machine }, sep: true }
      : { label: "Remove helper: choose Ask in Flockbook settings first…", act: "settings", sep: true }) : null,
    { label: "Put away", cmd: "tab.settle", args: { id: tab.id }, sep: true },
    { label: "Close tab", cmd: "tab.close", args: { id: tab.id }, danger: true },
  ];
}

export function spaceItems(space) {
  return [
    { label: "New tab here", cmd: "tab.new", args: { space: space.id } },
    { label: "Rename…", cmd: "space.rename", args: { id: space.id } },
    { label: "Close workspace…", cmd: "space.close", args: { id: space.id }, danger: true, sep: true },
  ];
}

export class MenuView {
  constructor(_cx) {}

  render(_cx) {
    const t = theme();
    const p = props() ?? {};
    const rows = [];
    for (const [i, item] of (p.items ?? []).entries()) {
      if (item.sep) rows.push(div().h(1).mx(6).my(4).bg(t.border));
      rows.push(h_flex().id(`menu-${i}`).h(ROW_H).px(10).items_center().rounded(6).text_size(13)
        .text_color(item.danger ? t.blocked : t.foreground).hover((s) => s.bg(t.hover))
        .on_click(() => { closeOverlay(p.overlay); act(item); })
        .child(div().truncate().child(item.label)));
    }
    return v_flex().size_full().p(PAD).gap(0).children(rows);
  }
}
