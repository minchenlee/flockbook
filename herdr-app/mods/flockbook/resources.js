// Resources: the docs, artifacts, PRs, commands and links the shown agent pointed you to in its replies (the
// helper's `resources` command, docs/protocol.md "res"). A window under the bar's right edge (⌘⇧R or
// the bar's button); the button's badge counts the ones newer than you last saw.
//
// Kept in shared() (persisted), so every copy of the mod and a restart see the same:
//   flockbook.res.data        { terminal, cursor, items, at }: the last answer, for the shown agent only
//   flockbook.res.on          the helper has the command (older helpers: no button)
//   flockbook.res.open        the open window's overlay id; flockbook.res.alive: when it last drew (ms)
//   flockbook.res.frame       where it was opened (placement()): the chat makes room for it
//   flockbook.res.scope       "agent" (P1 has only "This agent")
//   flockbook.res.seen.<t>    the newest last_ts seen in the open window; set to the newest one the first
//                             time an agent is seen, so an old conversation does not light the badge
//   flockbook.res.removed.<t> [{ id, last_ts }] taken off the list; one mentioned again later
//                             (a newer last_ts) comes back. A bare id string is also read (removed for good).
//   flockbook.res.jump        { terminal, row, at }: the chat scrolls to that row once
import { div, svg } from "gpui-kit";
import { v_flex, h_flex } from "gpui-base";
import { theme, props, shared, setShared, openOverlay, closeOverlay, exec, window } from "herdr";
import { currentAgent, machineOf, icon, showScreen, sidebarShown } from "./state.js";
import * as sidecar from "./sidecar.js";
import { READ_W, GUTTER, columnWidth } from "./estimate.js";

export const KEY_DATA = "flockbook.res.data";
export const KEY_ON = "flockbook.res.on";
export const KEY_OPEN = "flockbook.res.open";
export const KEY_ALIVE = "flockbook.res.alive";
export const KEY_SCOPE = "flockbook.res.scope";
export const KEY_JUMP = "flockbook.res.jump";
const KEY_POLL_AT = "flockbook.res.polled";
export const KEY_WIN_W = "flockbook.window.width";
export const KEY_SIDE_W = "flockbook.sidebar.width";
export const seenKey = (t) => `flockbook.res.seen.${t}`;
export const removedKey = (t) => `flockbook.res.removed.${t}`;
export const term = (address) => String(address ?? "").replace(/^hc:/, "");

export const RES_MS = 5000;           // one ask at most this often, over every copy
export const PANEL_W = 320;
export const READ_GAP = 24;           // at least this between the window and the chat's column and input
export const NARROW_W = 720;          // below it: a drawer down the right side
const GAP = 12, TOP = 60;             // under the 56 px bar
const MIN_COL = 240;                  // the reading column's least width (estimate.js columnWidth)
const INPUT_W = 800, INPUT_SIDE = 16; // the chat input: max_w(800) inside px(16) (composer.js)
const REFIT_PX = 8;                   // a smaller change in height does not reopen the window
export const KEY_FRAME = "flockbook.res.frame";
const ALIVE_EVERY = 2000;             // the open window says it is there this often
const ALIVE_MS = 6000;                // a window that has not said so for this long is gone (a restart)

export const GROUPS = [
  { id: "docs", label: "Docs", kinds: ["doc"] },
  { id: "artifacts", label: "Artifacts", kinds: ["artifact"] },
  { id: "github", label: "PRs & CI", kinds: ["pr", "issue", "ci"] },
  { id: "commands", label: "Commands", kinds: ["cmd"] },
  { id: "links", label: "Links", kinds: ["link", "local"] },
];
// The row actions' icons (Lucide, ISC: RECIPE.md "Art").
export const ACT_ICONS = { jump: "message", copy: "copy", remove: "eye-off" };
const ICON = { doc: "doc", artifact: "grid", pr: "branch", issue: "branch", ci: "refresh", local: "terminal", link: "link", cmd: "terminal" };
export const OPENS = ["md", "html", "htm", "pdf", "png", "svg"];

const byNewest = (a, b) => (b.last_ts ?? -Infinity) - (a.last_ts ?? -Infinity);

// The next data from an answer: all of it when `full` (or another agent), else the new lines merged in by id.
// An answer with an error keeps what was there.
export function applyAnswer(prev, terminal, answer, at = Date.now()) {
  if (answer?.error || !answer?.end) return prev ?? null;
  const lines = (answer.lines ?? []).filter((l) => l.type === "res" && l.id);
  const full = answer.end.full !== false || !prev || prev.terminal !== terminal;
  let items;
  if (full) items = lines;
  else {
    const by = new Map(prev.items.map((i) => [i.id, i]));
    for (const l of lines) by.set(l.id, l);
    items = [...by.values()];
  }
  items = items.map(({ type: _t, ...rest }) => rest).sort(byNewest);
  return { terminal, cursor: answer.end.cursor ?? null, items, at };
}

export const newest = (items) => (items ?? []).reduce((m, i) => Math.max(m, i.last_ts ?? 0), 0);

// The removed list as [{ id, last_ts }] (a bare id: last_ts null, removed for good).
export function removedOf(list) {
  return (Array.isArray(list) ? list : []).map((r) => (typeof r === "string" ? { id: r, last_ts: null } : r)).filter((r) => r?.id);
}

// Removed and not mentioned since.
export function isRemoved(item, removed) {
  const r = removed.find((x) => x.id === item.id);
  return Boolean(r) && (r.last_ts === null || r.last_ts === undefined || (item.last_ts ?? 0) <= r.last_ts);
}

export function visible(items, removed) {
  const rm = removedOf(removed);
  return (items ?? []).filter((i) => !isRemoved(i, rm));
}

export const isNew = (item, seen) => seen !== null && seen !== undefined && (item.last_ts ?? 0) > seen;

export function badgeCount(items, seen, removed) {
  return visible(items, removed).filter((i) => isNew(i, seen)).length;
}

export function grouped(items) {
  return GROUPS.map((g) => ({ ...g, items: items.filter((i) => g.kinds.includes(i.kind)) })).filter((g) => g.items.length);
}

// "now", "4m", "3h", "2d".
export function ago(ms, now = Date.now()) {
  if (!ms) return "";
  const s = Math.max(0, (now - ms) / 1000);
  if (s < 60) return "now";
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  if (s < 86400) return `${Math.floor(s / 3600)}h`;
  return `${Math.floor(s / 86400)}d`;
}

// What a click does: { copy } for a command (copied, never run); { url } for http(s); { cmd, args } for a file this computer may open (a doc of the
// kinds above, listed as there, an absolute path, an agent on this computer; openItem asks the helper
// again); else { why }.
export function openPlan(item, address, platform = window()?.platform) {
  if (!item) return { why: "Nothing to open." };
  // A command is copied, never run.
  if (item.kind === "cmd") return typeof item.target === "string" && item.target ? { copy: item.target } : { why: "Nothing to copy." };
  if (item.kind !== "doc") {
    return /^https?:\/\//i.test(item.target ?? "") ? { url: item.target } : { why: "Only web addresses open from here." };
  }
  const path = String(item.target ?? "");
  const m = machineOf(address);
  if (m) return { why: `This file is on ${m}. Open it there.` };
  if (!path.startsWith("/")) return { why: "Not a full path." };
  const ext = (path.split("/").pop().split(".").pop() ?? "").toLowerCase();
  if (!OPENS.includes(ext)) return { why: "Flockbook does not open this kind of file." };
  if (item.exists !== true) return { why: "File not found." };
  return { cmd: platform === "macos" ? "open" : "xdg-open", args: [path] };
}

// Opens it. A file: the helper looks at it now (`open-check`: links followed, the real file must be a kind
// above) and the real file opens; `exists` in the list may be old. `run`: how a file opens (tests pass their
// own). Resolves the plan that ran, or { why }.
export async function openItem(item, address, cx, run = exec) {
  const p = openPlan(item, address);
  if (p.url) { cx?.open_url?.(p.url); return p; }
  if (p.copy) { cx?.write_to_clipboard?.(p.copy); return p; }
  if (!p.cmd) return p;
  if (!(await sidecar.hasCommand("open-check"))) return { why: "Update the Flockbook helper to open files." };
  const a = await sidecar.openCheck(p.args[0]);
  const o = a.lines?.find((l) => l.type === "openable");
  if (a.error || !o) return { why: a.error?.message ?? "Could not look at the file." };
  if (o.ok !== true || typeof o.path !== "string" || !o.path.startsWith("/")) return { why: o.why || "Flockbook does not open this file." };
  const plan = { cmd: p.cmd, args: [o.path] };
  run(plan.cmd, plan.args);
  return plan;
}

export function removeItem(item, terminal) {
  const list = removedOf(shared(removedKey(terminal))).filter((r) => r.id !== item.id);
  list.push({ id: item.id, last_ts: item.last_ts ?? null });
  setShared(removedKey(terminal), list);
}

export function jumpTo(item, terminal) {
  setShared(KEY_JUMP, { terminal, row: item.row, at: Date.now() });
  showScreen("chat");
}

// The data for this agent, when it is the one asked about last.
export function dataFor(address) {
  const d = shared(KEY_DATA);
  return d && address && d.terminal === term(address) ? d : null;
}

// The bar's badge for the shown agent.
export function badgeFor(address) {
  const d = dataFor(address);
  if (!d) return 0;
  const t = term(address);
  return badgeCount(d.items, shared(seenKey(t)), shared(removedKey(t)));
}

// One `resources` for the shown agent, at most each RES_MS over every copy (`force`: now). Redraws
// only when something changed.
let busy = false, last = 0;
export const forgetResources = () => { busy = false; last = 0; };
// `full`: no cursor, so docs are looked at again (a file written after it was mentioned, a new title).
export async function pollResources(cx, force = false, full = false) {
  const address = currentAgent()?.address ?? null;
  if (!address || busy) return;
  const t = term(address);
  const was = shared(KEY_DATA);
  const same = was?.terminal === t;
  if (!force && same && Date.now() - Math.max(last, shared(KEY_POLL_AT) ?? 0) < RES_MS) return;
  busy = true; last = Date.now(); setShared(KEY_POLL_AT, last);
  try {
    const on = await sidecar.hasCommand("resources");
    if (shared(KEY_ON) !== on) { setShared(KEY_ON, on); cx?.notify?.(); }
    if (!on) return;
    const r = await sidecar.resources(address, same && !full ? was.cursor : null);
    const next = applyAnswer(same && !full ? was : null, t, r);
    if (!next) return;
    if (shared(seenKey(t)) === undefined || shared(seenKey(t)) === null) setShared(seenKey(t), newest(next.items));
    if (!same || JSON.stringify(was.items) !== JSON.stringify(next.items) || was.cursor !== next.cursor) {
      setShared(KEY_DATA, next); cx?.notify?.();
    }
  } finally { busy = false; }
}

// The least right inset so a box at most `maxW` wide, centered between `side` and the pane's right edge less
// the inset and `side`, ends at `right` px from the pane's right edge or before.
function insetFor(paneW, right, maxW, side) {
  const x = Math.max(0, Math.ceil(paneW + maxW - 2 * (paneW - right)));
  return paneW - 2 * side - x >= maxW ? x : Math.max(0, right - side);
}
// How far the chat moves its content left from the pane's right edge for a window `right` px wide there.
const insetBeside = (paneW, right) => Math.max(insetFor(paneW, right, READ_W, GUTTER), insetFor(paneW, right, INPUT_W, INPUT_SIDE));

// Where the window goes: under the bar's right edge, or a drawer down the right side of a narrow window.
// `paneW`: the chat's width (the window less the sidebar). A drawer too when the chat has no room for its
// column's least width beside the window. `right`: px of the chat's right side the window covers (0: a drawer).
export function placement(winW, winH, count = 0, groups = 0, paneW = winW) {
  const h = winH > 0 ? winH : 800;
  const w = winW > 0 ? winW : 1200;
  const pane = paneW > 0 ? paneW : w;
  const right = PANEL_W + GAP;
  const tight = pane - insetBeside(pane, right + READ_GAP) - 2 * GUTTER < MIN_COL;
  if (w < NARROW_W || tight) return { at: { x: Math.max(0, w - PANEL_W), y: 0 }, width: PANEL_W, height: h, drawer: true, right: 0 };
  const want = 46 + 34 + Math.max(1, count) * 50 + groups * 30 + 16;
  return { at: { x: Math.max(0, w - right), y: TOP }, width: PANEL_W, height: Math.max(180, Math.min(h - 72, count ? want : 200)), drawer: false, right };
}

// The open window's frame, else null.
export const openFrame = () => (panelOpen() ? shared(KEY_FRAME) ?? null : null);

// The chat's reading column beside the window (`frame`, from placement(); null: none open): `left` and `width` of
// the column, `inset` px the content keeps free on the right, `composerRight` where the input ends (pane px).
// Room enough: as with no window. Else the column moves left, and narrows only as far as it must.
export function readingLayout(paneW, frame = openFrame()) {
  const w = paneW || 800;
  const right = frame && !frame.drawer && frame.right > 0 ? frame.right + READ_GAP : 0;
  const inset = right ? insetBeside(w, right) : 0;
  const width = columnWidth(w - inset);
  const a = w - inset - 2 * INPUT_SIDE;
  return { inset, left: Math.max(GUTTER, Math.floor((w - inset - width) / 2)), width, composerRight: INPUT_SIDE + (a + Math.min(INPUT_W, a)) / 2 };
}

// Whether the open window (`prev`) must be opened again at `next`: another place or width, or a height
// REFIT_PX or more off. A smaller one waits while the pointer is on it (a removed row would move the rest).
export function needsRefit(prev, next, inside = false) {
  if (!prev || !next) return false;
  if (prev.drawer !== next.drawer || prev.at?.x !== next.at?.x || prev.at?.y !== next.at?.y || prev.width !== next.width) return true;
  const d = (next.height ?? 0) - (prev.height ?? 0);
  if (Math.abs(d) < REFIT_PX) return false;
  return d > 0 || !inside;
}

// Where the window goes now: the shown agent's items, the window's size.
export function placeNow() {
  const address = currentAgent()?.address ?? null;
  const d = dataFor(address);
  const items = d ? visible(d.items, shared(removedKey(term(address)))) : [];
  const paneW = shared(KEY_WIN_W) ?? 0;
  const winW = paneW + (sidebarShown() ? shared(KEY_SIDE_W) ?? 0 : 0);
  return placement(winW, shared("flockbook.window.height") ?? 0, items.length, grouped(items).length, paneW);
}

// Opens the window at `p`, then closes `old` (a refit): the new one is open before the old one goes, and the old
// one's close() sees that it is no longer the open one. No `key`: Herdr opens a keyed window where that key's
// window was (dropped there, or the one it replaces), not at `p.at`, and the chat makes room where the frame says.
function openPanel(p, extra = {}, old = null) {
  setShared(KEY_ALIVE, Date.now());
  setShared(KEY_FRAME, p);
  const id = openOverlay({ mod: "flockbook", props: { view: "resources", drawer: p.drawer, ...extra }, kind: "window", frame: "none",
    at: p.at, width: p.width, height: p.height });
  setShared(KEY_OPEN, id);
  if (old && old !== id) closeOverlay(old);
}

export const panelOpen = () => Boolean(shared(KEY_OPEN)) && Date.now() - (shared(KEY_ALIVE) ?? 0) < ALIVE_MS;

// ⌘⇧R and the bar's button: open the window, or close it when it is open.
export function toggleResources() {
  const open = shared(KEY_OPEN);
  if (open && panelOpen()) { setShared(KEY_OPEN, null); closeOverlay(open); return; }
  if (open) closeOverlay(open);   // left from before a restart
  setShared(KEY_SCOPE, shared(KEY_SCOPE) ?? "agent");
  openPanel(placeNow());
}

export class ResourcesView {
  constructor(cx) {
    this.h = cx?.focus_handle?.() ?? null;
    this.address = currentAgent()?.address ?? null;
    // Opened again to fit (refit()): the dots and the keyboard as they were, no full read.
    const p = props() ?? {};
    // The "new" dots: newer than what was seen when the window opened.
    this.seenAtOpen = p.refit ? p.seenAtOpen ?? null : this.address ? shared(seenKey(term(this.address))) ?? null : null;
    this.inside = false; // the pointer is on the window
    this.note = null;   // why the last click did not open
    this.hover = null;  // the row the pointer is on: its actions show
    cx?.spawn?.((cx) => (p.refit ? pollResources(cx) : pollResources(cx, true, true)));
    cx?.timer?.every?.(1000, (cx) => this.tick(cx));
    // A window does not take the keyboard when it opens: focus it, after the first frame, so Esc reaches it.
    if (!p.refit || p.focus) cx?.timer?.after?.(16, () => this.h?.focus?.());
  }

  tick(cx) {
    // Not every second: each set redraws every copy.
    if (Date.now() - (shared(KEY_ALIVE) ?? 0) >= ALIVE_EVERY) setShared(KEY_ALIVE, Date.now());
    const a = currentAgent()?.address ?? null;
    if (a !== this.address) { this.address = a; this.seenAtOpen = a ? shared(seenKey(term(a))) ?? null : null; this.note = null; cx?.notify?.(); }
    cx?.spawn?.((cx) => pollResources(cx));
    this.markSeen();
    this.refit();
  }

  // The item count, the shown agent or the window's size changed: open it again at its new size (no way to
  // resize an open window). At most once a tick.
  refit() {
    if (!panelOpen()) return;
    const p = placeNow();
    if (!needsRefit(shared(KEY_FRAME), p, this.inside || this.hover !== null)) return;
    openPanel(p, { refit: true, focus: Boolean(this.h?.is_focused?.()), seenAtOpen: this.seenAtOpen }, props()?.overlay ?? shared(KEY_OPEN));
  }

  // Shown is seen: the badge goes.
  markSeen() {
    const d = dataFor(this.address);
    if (!d) return;
    const k = seenKey(term(this.address)), n = newest(d.items);
    if ((shared(k) ?? 0) < n) setShared(k, n);
  }

  closeSelf() {
    const id = props()?.overlay ?? shared(KEY_OPEN);
    if (shared(KEY_OPEN)) setShared(KEY_OPEN, null);
    if (id) closeOverlay(id);
  }

  // Only this window's own id is cleared: a refit opens the new one before this one closes.
  close(_cx) { const id = props()?.overlay; if (id && shared(KEY_OPEN) === id) setShared(KEY_OPEN, null); }

  render(cx) {
    const t = theme();
    const stop = (_e, cx) => cx?.stop_propagation?.();
    const address = this.address, tk = term(address);
    const d = dataFor(address);
    const removed = address ? shared(removedKey(tk)) : [];
    const items = d ? visible(d.items, removed) : [];
    const hidden = d ? d.items.length - items.length : 0;
    const now = Date.now();
    const iconBtn = (id, name, tip, action) => div().id(id).size(24).flex_none().flex().items_center().justify_center().rounded(6)
      .text_color(t.muted).hover((s) => s.bg(t.hover).text_color(t.foreground)).tooltip(tip)
      .on_mouse_down("left", stop).on_click((e, cx) => { cx?.stop_propagation?.(); action(e, cx); }).child(svg(icon(name)).size(13));

    const header = h_flex().id("res-head").h(46).flex_none().px(14).gap(8).items_center().border_b(1).border_color(t.border)
      .child(div().text_size(13).font_weight(600).text_color(t.foreground).child("Resources"))
      .child(div().id("res-scope").h(22).px(8).flex().items_center().rounded(11).bg(t.selected).text_size(11.5).text_color(t.muted).child("This agent"))
      .child(div().flex_1())
      .child(iconBtn("res-close", "close", "Close (Esc)", () => this.closeSelf()));

    const row = (it) => {
      const gone = it.kind === "doc" && it.exists !== true;
      const fresh = isNew(it, this.seenAtOpen);
      const sub = gone ? `${it.detail} · File not found` : it.detail;
      return h_flex().id(`res-${it.id}`).relative().px(10).py(7).mx(4).gap(10).items_start().rounded(8)
        .hover((s) => s.bg(t.hover))
        .on_hover((inside, cx) => { const was = this.hover; this.hover = inside ? it.id : this.hover === it.id ? null : this.hover; if (was !== this.hover) cx?.notify?.(); })
        .on_mouse_down("left", stop)
        .on_click((_e, cx) => {
          // Started here: a web address opens with the click's cx, before the first await.
          const pending = openItem(it, address, cx);
          cx?.spawn?.(async (acx) => { const p = await pending; this.note = p.why ? { id: it.id, text: p.why } : p.copy ? { id: it.id, text: "Copied.", ok: true } : null; acx?.notify?.(); });
        })
        .when(fresh, (el) => el.child(div().id(`res-new-${it.id}`).absolute().left(1).top(16).size(6).rounded(3).bg(t.accent)))
        .child(div().size(22).flex_none().mt(1).rounded(6).bg(t.raised).flex().items_center().justify_center()
          .when(gone, (el) => el.opacity(0.5)).child(svg(icon(ICON[it.kind] ?? "link")).size(12).text_color(t.muted)))
        .child(v_flex().flex_1().min_w(0).gap(1)
          .child(h_flex().gap(6).items_center()
            .child(div().flex_1().min_w(0).truncate().text_size(13).font_weight(550).text_color(gone ? t.dim : t.foreground)
              .when(gone, (el) => el.line_through()).child(it.title || it.target))
            .child(div().flex_none().text_size(11).text_color(t.dim).child(ago(it.last_ts, now))))
          .child(div().truncate().text_size(11.5).text_color(t.muted).when(it.kind === "doc" || it.kind === "cmd", (el) => el.font_family("mono")).child(sub || it.target))
          .when(this.note?.id === it.id, (el) => el.child(div().text_size(11.5).text_color(this.note.ok ? t.muted : t.blocked).child(this.note.text))))
        .when(this.hover === it.id, (el) => el.child(h_flex().id(`res-acts-${it.id}`).gap(2).flex_none()
          .child(iconBtn(`res-jump-${it.id}`, ACT_ICONS.jump, "Show in chat", () => jumpTo(it, tk)))
          // A command's click copies it already.
          .when(it.kind !== "cmd", (a) => a.child(iconBtn(`res-copy-${it.id}`, ACT_ICONS.copy, "Copy", (_e, cx) => cx?.write_to_clipboard?.(it.target))))
          .child(iconBtn(`res-remove-${it.id}`, ACT_ICONS.remove, "Remove from list", (_e, cx) => { removeItem(it, tk); cx?.notify?.(); }))));
    };

    const body = items.length
      ? v_flex().id("res-list").flex_1().min_h(0).py(4).overflow_y_scrollbar().children(grouped(items).map((g) => v_flex().id(`res-group-${g.id}`)
        .child(h_flex().px(14).pt(8).pb(3).items_center()
          .child(div().flex_1().text_size(11).font_weight(600).text_color(t.muted).child(g.label))
          .child(div().text_size(11).font_weight(600).text_color(t.muted).child(String(g.items.length))))
        .children(g.items.map(row))))
      : v_flex().id("res-empty").flex_1().px(24).py(28).gap(6).items_center().justify_center()
        .child(div().text_size(13).font_weight(600).text_color(t.foreground).child(address ? "No resources yet" : "No agent here"))
        .child(div().text_size(12).text_color(t.muted).text_center()
          .child(address ? "Docs, artifacts and links the agent mentions in its replies show up here." : "Open an agent's tab to see what it pointed you to."));

    const foot = h_flex().id("res-foot").h(34).flex_none().px(14).items_center().border_t(1).border_color(t.border)
      .text_size(11.5).text_color(t.muted)
      .child(`${items.length} item${items.length === 1 ? "" : "s"}${hidden ? ` · ${hidden} removed` : ""}`);

    const root = v_flex().id("res-root").size_full().bg(t.background).border(1).border_color(t.border).shadow_lg()
      .when(props()?.drawer !== true, (el) => el.rounded(14))
      .on_hover((inside) => { this.inside = Boolean(inside); })
      // A press it does not stop drags the window: every press is stopped, so it stays where the chat made room.
      .on_mouse_down("left", (_e, cx) => { cx?.stop_propagation?.(); this.h?.focus?.(); })
      .on_key_down((e, cx) => { if (e?.key === "escape") { cx?.stop_propagation?.(); this.closeSelf(); } });
    if (this.h) root.track_focus(this.h);
    return root.child(header).child(body).child(foot);
  }
}
