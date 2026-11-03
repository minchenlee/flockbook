// What every view agrees on: the agent shown in the chat, and the screen in the stack.
// Kept in shared() so the copies of this mod (one per layout block) see the same value.
import { agents, focused, shared, setShared, run, spaces, openOverlay, closeOverlay, appHome, readTextFile } from "herdr";

export const KEY_AGENT = "flockbook.agent";
// The screen in the stack, as this mod last selected it (layout() is the file, not the live tree).
export const KEY_SCREEN = "flockbook.screen";
export const shownScreen = () => shared(KEY_SCREEN) ?? "chat";
export function showScreen(child) {
  setShared(KEY_SCREEN, child);
  run("layout.select", { stack: "screen", child });
}
const MOD = "mods/flockbook";
export const icon = (name) => `${MOD}/icons/${name}.svg`;

// One spinner speed everywhere (ms per turn).
export const SPIN_MS = 600;

// Motion: a quick ease-out for fades and a soft spring for moves, as in the web app.
export const FADE = { duration: 180, easing: "ease-out" };
export const SPRING = { response: 320, damping: 0.82 };

// Whether the sidebar shows. layout() gives the file, not the live tree, so the mod keeps its own
// copy: read once from the app's saved layout state, then set by toggleSidebar().
export const KEY_SIDEBAR = "flockbook.sidebar";
export const sidebarShown = () => shared(KEY_SIDEBAR) === true;
export function toggleSidebar() {
  const show = !sidebarShown();
  endPeek();
  setShared(KEY_SIDEBAR, show);
  run(show ? "layout.show" : "layout.hide", { id: "sidebar" });
}
// The pointer at the window's left edge floats the sidebar over the content (a frameless popup that
// the side view draws as a panel), so the content does not move. It closes when the pointer leaves
// it, on Esc or on a click beside it. KEY_PEEK holds the popup's id while it is open.
export const KEY_PEEK = "flockbook.sidebar.peek";
export const EDGE_PX = 4;
export const PEEK_W = 260, PEEK_GAP = 8;
// A popup is as tall as its content unless given px, so the views note the window's height.
export const KEY_WIN_H = "flockbook.window.height";
export function noteWindowHeight(size) {
  const h = Math.round(size?.height ?? 0);
  if (h > 0 && shared(KEY_WIN_H) !== h) setShared(KEY_WIN_H, h);
}
export function peekAtEdge(e) {
  if (sidebarShown() || shared(KEY_PEEK) || (e?.position?.x ?? 99) > EDGE_PX) return;
  const id = openOverlay({ mod: "flockbook", props: { view: "side", peek: true }, kind: "popup", frame: "none",
    at: { x: PEEK_GAP, y: PEEK_GAP }, width: PEEK_W, height: (shared(KEY_WIN_H) ?? 800) - PEEK_GAP * 2 });
  setShared(KEY_PEEK, id);
}
export function endPeek() {
  const id = shared(KEY_PEEK);
  if (!id) return;
  setShared(KEY_PEEK, null);
  closeOverlay(id);
}
// The panel went (Esc, a click beside it, or endPeek): forget it so the edge opens it again.
export function peekClosed(id) {
  if (shared(KEY_PEEK) === id) setShared(KEY_PEEK, null);
}
// Adds the edge watch to a view's root.
export const watchEdge = (el) => el.on_mouse_move((e) => peekAtEdge(e));

export async function loadSidebar() {
  if (shared(KEY_SIDEBAR) !== undefined) return;
  let hidden = true;
  try {
    const text = await readTextFile(`${appHome()}/layouts/.state/flockbook.json`);
    const v = text ? JSON.parse(text)?.nodes?.sidebar?.hidden : undefined;
    if (typeof v === "boolean") hidden = v;
  } catch { /* no saved state: the file's default */ }
  if (shared(KEY_SIDEBAR) === undefined) setShared(KEY_SIDEBAR, !hidden);
}

// The agents of the shown space, in herdr's order.
export function spaceAgents() {
  const space = spaces().find((s) => s.active);
  const all = agents();
  return space ? all.filter((a) => a.space === space.id) : all;
}

// The shown space, and its tabs that are not put away (tab.settle), in herdr's order.
export function activeSpace() { return spaces().find((s) => s.active) ?? null; }
export function spaceTabs() { return (activeSpace()?.tabs ?? []).filter((t) => !t.settled); }
export function activeTab() { return activeSpace()?.tabs?.find((t) => t.active) ?? null; }
export function tabAgents(id) { return agents().filter((a) => a.tab === id); }

// The agent the chat shows: an agent of the shown tab (the one picked here, else the focused one,
// else the first). Null when that tab has no agent. Without tab data, any agent of the space.
export function currentAgent() {
  const tab = activeTab();
  const list = tab ? tabAgents(tab.id) : spaceAgents();
  const picked = shared(KEY_AGENT);
  const f = focused();
  return list.find((a) => a.address === picked)
    ?? list.find((a) => f && (a.address === f.agent || a.address === f.terminal))
    ?? list[0] ?? null;
}

// A tab's label: its name when someone named it, else its agent's name, else its number ("3").
export function tabLabel(tab) {
  if (tab.named) return tab.name;
  const a = tabAgents(tab.id)[0];
  return a ? agentName(a) : tab.name;
}

// The screen for a tab: the chat when an agent runs in it, else the tab's own terminals.
// The chat for a tab with an agent in one pane; the tab's panes once it is split (or for a shell).
export const screenFor = (tab) => (tab && tabAgents(tab.id).length && (tab.panes ?? 1) <= 1 ? "chat" : "tab");

export function showTab(tab) {
  setShared(KEY_AGENT, null);
  run("tab.select", { id: tab.id });
  showScreen(screenFor(tab));
}

// Closes a tab (herdr asks first when it is the space's last tab).
export function closeTab(id) {
  run("tab.close", { id });
}
export const closeAgent = (a) => closeTab(a.tab);

// A label for people: the tab's name when someone named it, else the program ("Claude Code").
export function agentName(a) {
  if (!a) return "";
  if (a.tab_name && !/^\d+$/.test(a.tab_name) && !/^tb\d+$/.test(a.tab_name)) return a.tab_name;
  return a.program?.name ?? a.app ?? "Agent";
}

export function showAgent(address) {
  setShared(KEY_AGENT, address);
  run("agent.focus", { terminal: address });
  showScreen("chat");
}

export function showNeeds() { showScreen("needs"); }
export function showOverview() { showScreen("overview"); }

// Opens a tab of any space: that space, that tab, and its screen.
export function openTab(spaceId, tab) {
  setShared(KEY_AGENT, null);
  run("space.select", { id: spaceId });
  run("tab.select", { id: tab.id });
  showScreen(screenFor(tab));
}

// Agents that wait for the owner, in every space.
export function needing() {
  return agents().filter((a) => a.status === "blocked");
}

// Shows a space's chat. The pick is cleared, so the chat starts at the space's focused or first agent.
export function showSpace(id) {
  setShared(KEY_AGENT, null);
  run("space.select", { id });
  showScreen("chat");
}

// The agent's own terminal in a floating window, for anything the chat cannot do. A title bar
// (view "termbar") closes it, because Esc goes to the terminal. Opening it again replaces it.
export const KEY_TERM = "flockbook.terminal";   // the open terminal window's overlay id

// ctrl-` and the Terminal button: open the agent's terminal window, or close it when it is open.
export function openTerminal(a) {
  const open = shared(KEY_TERM);
  if (open) { setShared(KEY_TERM, null); closeOverlay(open); return; }
  if (!a) return;
  const id = openOverlay({
    kind: "window", key: "flockbook-terminal", width: 0.7, height: 0.7,
    layout: { type: "column", children: [
      { type: "block", size: { px: 42 }, content: { mod: "flockbook", props: { view: "termbar", terminal: a.address, name: agentName(a) } } },
      { type: "block", content: { terminal: a.address }, padding: 10 },
    ] },
  });
  setShared(KEY_TERM, id);
}
