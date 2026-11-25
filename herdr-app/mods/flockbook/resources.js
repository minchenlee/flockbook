// Resources: the docs, artifacts, PRs, commands and links the shown agent pointed you to in its replies (the
// helper's `resources` command, docs/protocol.md "res"). A window under the bar's right edge (⌘⇧R or
// the bar's button); the button's badge counts the ones newer than you last saw.
//
// The lists themselves are in memory (`cache`, module code is shared by every copy): the last CACHE_MAX agents,
// { terminal, cursor, items, at, error? } each, so going back to an agent shows its list at once and asks only
// --since its cursor. Not persisted: after a restart the first ask is a full read (owner 11-25: 761 items made a
// 179 KB value written each change).
// Kept in shared() (persisted), so every copy of the mod and a restart see the same:
//   flockbook.res.data        { terminal, cursor, count, at, error?, v }: what changed last, small; `v` goes up at
//                             each change, so the other copies draw again. `error`: "old" or "missing", the helper on
//                             the agent's machine cannot list them
//   flockbook.res.collapsed   the ids of the folded groups (GROUPS), for every agent
//   flockbook.res.on          the helper has the command (older helpers: no button)
//   flockbook.res.open        the open window's overlay id; flockbook.res.run: the app run it was opened in (RUN).
//                             An id from another run (before a restart) is not open: no heartbeat, nothing written while open
//   flockbook.res.frame       where it was opened (placement()); the chat does not move for it
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
import { GUTTER, columnWidth } from "./estimate.js";

export const KEY_DATA = "flockbook.res.data";
export const KEY_ON = "flockbook.res.on";
export const KEY_OPEN = "flockbook.res.open";
export const KEY_RUN = "flockbook.res.run";
export const KEY_SCOPE = "flockbook.res.scope";
export const KEY_JUMP = "flockbook.res.jump";
export const KEY_COLLAPSED = "flockbook.res.collapsed";
export const KEY_WIN_W = "flockbook.window.width";
export const KEY_SIDE_W = "flockbook.sidebar.width";
export const seenKey = (t) => `flockbook.res.seen.${t}`;
export const removedKey = (t) => `flockbook.res.removed.${t}`;
export const term = (address) => String(address ?? "").replace(/^hc:/, "");

export const RES_MS = 5000;           // one ask at most this often, over every copy
export const CACHE_MAX = 8;           // agents whose list is kept in memory
export const ROW_LIMIT = 30;          // rows drawn a group before "Show N more"
export const PANEL_W = 320;
export const READ_GAP = 24;           // at least this between the window and the chat's column and input
export const NARROW_W = 720;          // below it: a drawer down the right side
const GAP = 12, TOP = 60;             // under the 56 px bar
export const INPUT_GUESS = 160;       // the chat input's height before chat.js has measured it (one line, status row: about 150)
const MIN_PANEL = 260;                // less room right of the column: PANEL_W wide over the chat
export const KEY_INPUT_H = "flockbook.input.height"; // the chat input's height (chat.js), for the window's bottom
const INPUT_W = 800, INPUT_SIDE = 16; // the chat input: max_w(800) inside px(16) (composer.js)
const REFIT_PX = 8;                   // a smaller change in height does not reopen the window
export const KEY_FRAME = "flockbook.res.frame";
// This run of the app: module code runs once per app start and is shared by every copy (herdr AGENTS.md), so a
// window opened in this run carries it; after a restart the id left in shared has another one.
export const RUN = `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`;

// A doc's group comes from its file's extension (any case; the helper says kind "doc" for all of them); a doc of
// another kind of file is in Other docs. `icon`: its rows' icon.
export const GROUPS = [
  { id: "markdown", label: "Markdown", kinds: ["doc"], exts: ["md"], icon: "doc" },
  { id: "web", label: "Web pages", kinds: ["doc"], exts: ["html", "htm"], icon: "globe" },
  { id: "pdfs", label: "PDFs", kinds: ["doc"], exts: ["pdf"], icon: "file-text" },
  { id: "images", label: "Images", kinds: ["doc"], exts: ["png", "svg"], icon: "image" },
  { id: "otherdocs", label: "Other docs", kinds: ["doc"], icon: "doc" },
  { id: "artifacts", label: "Artifacts", kinds: ["artifact"] },
  { id: "github", label: "PRs & CI", kinds: ["pr", "issue", "ci"] },
  { id: "commands", label: "Commands", kinds: ["cmd"] },
  { id: "links", label: "Links", kinds: ["link", "local"] },
];
// The row actions' icons (Lucide, ISC: RECIPE.md "Art").
export const ACT_ICONS = { jump: "message", copy: "copy", remove: "eye-off" };
const ICON = { doc: "doc", artifact: "grid", pr: "branch", issue: "branch", ci: "refresh", local: "terminal", link: "link", cmd: "terminal" };
export const OPENS = ["md", "html", "htm", "pdf", "png", "svg"];

// "md" for "/p/Plan.MD"; "" without one (a dot in a folder or at the start of the name is not one).
export function extOf(target) {
  const name = String(target ?? "").split(/[\\/]/).pop();
  const i = name.lastIndexOf(".");
  return i > 0 ? name.slice(i + 1).toLowerCase() : "";
}
const BY_EXT = new Map(GROUPS.flatMap((g) => (g.exts ?? []).map((x) => [x, g.id])));
const BY_KIND = new Map(GROUPS.filter((g) => !g.exts && g.id !== "otherdocs").flatMap((g) => g.kinds.map((k) => [k, g.id])));
// The id of the group an item is in (null: none).
export function groupOf(item) {
  if (item?.kind === "doc") return BY_EXT.get(extOf(item.target)) ?? "otherdocs";
  return BY_KIND.get(item?.kind) ?? null;
}
const GROUP_ICON = new Map(GROUPS.map((g) => [g.id, g.icon]));
const iconOf = (item) => (item.kind === "doc" ? GROUP_ICON.get(groupOf(item)) : ICON[item.kind]) ?? "link";

const byNewest = (a, b) => (b.last_ts ?? -Infinity) - (a.last_ts ?? -Infinity);

// The next data from an answer: all of it when `full` (or another agent), else the new lines merged in by id.
// An answer with an error keeps what was there.
// The helper on the agent's machine cannot list them: an older one (no `resources` there) or none. Shown as a hint
// in place of "No resources yet". Another error keeps what was there. A helper there from before `resources` has
// the same protocol and version, so the helper here forwards its answer as it is: `usage` (an unknown command). Here
// `usage` is not that: the version line said this helper has the command.
const HELPER_HINT = { remote_helper_outdated: "old", remote_helper_missing: "missing" };
export function applyAnswer(prev, terminal, answer, at = Date.now()) {
  const code = answer?.error?.code;
  const hint = HELPER_HINT[code] ?? (code === "usage" && machineOf(terminal) ? "old" : undefined);
  if (hint) return prev?.terminal === terminal && prev.error === hint ? prev : { terminal, cursor: null, items: [], at, error: hint };
  if (answer?.error || !answer?.end) return prev ?? null;
  const lines = (answer.lines ?? []).filter((l) => l.type === "res" && l.id);
  const full = answer.end.full !== false || !prev || prev.terminal !== terminal;
  // Nothing new: the same list (the same array, so a compare is one ===).
  if (!full && !lines.length) {
    const cursor = answer.end.cursor ?? null;
    return cursor === prev.cursor && !prev.error ? prev : { terminal, cursor, items: prev.items, at };
  }
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

// Whether two lists draw the same: the same items, in the same order, with the same fields.
export function sameItems(a, b) {
  if (a === b) return true;
  if (!a || !b || a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) {
    const x = a[i], y = b[i];
    if (x === y) continue;
    const kx = Object.keys(x);
    if (kx.length !== Object.keys(y).length) return false;
    for (const k of kx) {
      if (x[k] === y[k]) continue;
      if (typeof x[k] !== "object" || JSON.stringify(x[k]) !== JSON.stringify(y[k])) return false;
    }
  }
  return true;
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
  const by = new Map(GROUPS.map((g) => [g.id, []]));
  for (const i of items) by.get(groupOf(i))?.push(i);
  return GROUPS.map((g) => ({ ...g, items: by.get(g.id) })).filter((g) => g.items.length);
}

// The groups as the window draws them: `shown` the rows drawn (none when folded; ROW_LIMIT unless the group is in
// `expanded`), `rest` the ones under "Show N more". `collapsed`, `expanded`: group ids. Both the window and its
// height use it; a filter (by time, later) goes before it.
export function layoutGroups(items, collapsed = [], expanded = [], limit = ROW_LIMIT) {
  const folded = new Set(Array.isArray(collapsed) ? collapsed : []), open = new Set(expanded ?? []);
  return grouped(items).map((g) => {
    if (folded.has(g.id)) return { ...g, collapsed: true, shown: [], rest: 0 };
    const shown = open.has(g.id) ? g.items : g.items.slice(0, limit);
    return { ...g, collapsed: false, shown, rest: g.items.length - shown.length };
  });
}

// Folds the group, or unfolds it (for every agent; kept over a restart).
export function toggleGroup(id) {
  const now = Array.isArray(shared(KEY_COLLAPSED)) ? shared(KEY_COLLAPSED) : [];
  setShared(KEY_COLLAPSED, now.includes(id) ? now.filter((x) => x !== id) : [...now, id]);
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

// The lists in memory, by terminal, oldest first (a Map keeps the order things were put in).
const cache = new Map();
function remember(d) {
  cache.delete(d.terminal); cache.set(d.terminal, d);
  while (cache.size > CACHE_MAX) cache.delete(cache.keys().next().value);
}
// Keeps `d` for its agent and says so in shared (small: no items), so every copy draws again.
export function putData(d) {
  remember(d);
  const s = { terminal: d.terminal, cursor: d.cursor ?? null, count: d.items?.length ?? 0, at: d.at, v: (shared(KEY_DATA)?.v ?? 0) + 1 };
  if (d.error) s.error = d.error;
  setShared(KEY_DATA, s);
}

// The list of this agent, when it is in memory.
export function dataFor(address) {
  return address ? cache.get(term(address)) ?? null : null;
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
// `busy`: when the ask now running started (0: none). The task that asks can be dropped while it waits (the
// window that spawned it closed to fit), and then its `finally` never runs: an ask older than BUSY_MS no longer counts.
// An ask that comes back after a newer one started drops its answer: it would write older data over the newer.
export const BUSY_MS = 15000;
// `wantFull`: a full read was asked while another ask ran; it runs when that one ends (the window opened during
// the bar's delta: docs are looked at again all the same).
let busy = 0, last = 0, lastT = null, wantFull = false;
export const forgetResources = () => { busy = 0; last = 0; lastT = null; wantFull = false; };
// The lists in memory gone, as after a restart (tests).
export const forgetCache = () => cache.clear();
// `full`: no cursor, so docs are looked at again (a file written after it was mentioned, a new title).
export async function pollResources(cx, force = false, full = false) {
  const address = currentAgent()?.address ?? null;
  if (!address) return;
  if (busy && Date.now() - busy < BUSY_MS) { if (full) wantFull = true; return; }
  if (full) wantFull = false;
  const t = term(address);
  // The cursor only with the list it belongs to: after a restart, a full read.
  const was = cache.get(t) ?? null;
  const same = Boolean(was);
  if (!force && same && lastT === t && Date.now() - last < RES_MS) return;
  const mine = Date.now(); busy = mine; last = mine; lastT = t;
  try {
    const on = await sidecar.hasCommand("resources");
    if (busy !== mine) return;
    if (shared(KEY_ON) !== on) { setShared(KEY_ON, on); cx?.notify?.(); }
    if (!on) return;
    const r = await sidecar.resources(address, same && !full ? was.cursor : null);
    if (busy !== mine) return;
    const next = applyAnswer(same && !full ? was : null, t, r);
    if (!next) return;
    if (!next.error && (shared(seenKey(t)) === undefined || shared(seenKey(t)) === null)) setShared(seenKey(t), newest(next.items));
    // Drawn again only when the list or the hint changed: a new cursor alone is kept quietly.
    if (!same || (was.error ?? null) !== (next.error ?? null) || !sameItems(was.items, next.items)) {
      putData(next); cx?.notify?.();
    } else if (next !== was) remember(next);
    else remember(was);
  } finally {
    if (busy === mine) busy = 0;
    // Also after an early return above. An ask a newer one replaced leaves it to that one.
    if (wantFull && !busy) { wantFull = false; await pollResources(cx, true, true); }
  }
}

// Where the window goes: right of the chat's reading column, or a drawer down the right side of a narrow window.
// The chat never moves for it (owner 11-25): the window takes the room right of the column less READ_GAP and
// GAP, MIN_PANEL to PANEL_W wide; with less room it is PANEL_W wide over the chat. `paneW`: the chat's width (the
// window less the sidebar). `inputH`: the chat input's height (0: not measured yet, INPUT_GUESS); the window, a drawer
// too, ends GAP above it.
export function placement(winW, winH, count = 0, groups = 0, paneW = winW, inputH = 0) {
  const h = winH > 0 ? winH : 800;
  const w = winW > 0 ? winW : 1200;
  const pane = paneW > 0 ? Math.min(paneW, w) : w;
  const bottom = h - (inputH > 0 ? inputH : INPUT_GUESS) - GAP;
  if (w < NARROW_W) return { at: { x: Math.max(0, w - PANEL_W), y: 0 }, width: PANEL_W, height: Math.max(80, bottom), drawer: true, right: 0 };
  const r = readingLayout(pane, null);
  const room = w - (w - pane + r.left + r.width) - READ_GAP - GAP;
  const width = room >= MIN_PANEL ? Math.min(PANEL_W, room) : PANEL_W;
  const most = Math.max(80, bottom - TOP);
  const want = 46 + Math.max(1, count) * 50 + groups * 30 + 16;
  const height = Math.min(most, Math.max(180, Math.min(h - 72, count ? want : 200)));
  return { at: { x: Math.max(0, w - GAP - width), y: TOP }, width, height, drawer: false, right: width + GAP };
}

// The open window's frame, else null.
export const openFrame = () => (panelOpen() ? shared(KEY_FRAME) ?? null : null);

// The chat's reading column: `left` and `width` of the column, `inset` (always 0: the chat no longer makes room for
// the window), `composerRight` where the input ends (pane px). The same with the window open or closed.
export function readingLayout(paneW, _frame = null) {
  const w = paneW || 800;
  const width = columnWidth(w);
  const a = w - 2 * INPUT_SIDE;
  return { inset: 0, left: Math.max(GUTTER, Math.floor((w - width) / 2)), width, composerRight: INPUT_SIDE + (a + Math.min(INPUT_W, a)) / 2 };
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

// Where the window goes now: the shown agent's rows as drawn (layoutGroups: a "Show N more" row counts as a
// group title), the window's size. `expanded`: the open window's groups shown in full.
export function placeNow(expanded = []) {
  const address = currentAgent()?.address ?? null;
  const d = dataFor(address);
  const items = d ? visible(d.items, shared(removedKey(term(address)))) : [];
  const gs = layoutGroups(items, shared(KEY_COLLAPSED), expanded);
  const rows = gs.reduce((n, g) => n + g.shown.length, 0), titles = gs.length + gs.filter((g) => g.rest).length;
  const paneW = shared(KEY_WIN_W) ?? 0;
  const winW = paneW + (sidebarShown() ? shared(KEY_SIDE_W) ?? 0 : 0);
  return placement(winW, shared("flockbook.window.height") ?? 0, rows, titles, paneW, shared(KEY_INPUT_H) ?? 0);
}

// Opens the window at `p`, then closes `old` (a refit): the new one is open before the old one goes, and the old
// one's close() sees that it is no longer the open one. No `key`: Herdr opens a keyed window where that key's
// window was (dropped there, or the one it replaces), not at `p.at`.
function openPanel(p, extra = {}, old = null) {
  if (shared(KEY_RUN) !== RUN) setShared(KEY_RUN, RUN);
  setShared(KEY_FRAME, p);
  const id = openOverlay({ mod: "flockbook", props: { view: "resources", drawer: p.drawer, ...extra }, kind: "window", frame: "none",
    at: p.at, width: p.width, height: p.height });
  setShared(KEY_OPEN, id);
  if (old && old !== id) closeOverlay(old);
}

export const panelOpen = () => Boolean(shared(KEY_OPEN)) && shared(KEY_RUN) === RUN;

// ⌘⇧R and the bar's button: open the window, or close it when it is open.
export function toggleResources() {
  const open = shared(KEY_OPEN);
  if (open && panelOpen()) { setShared(KEY_OPEN, null); closeOverlay(open); return; }
  if (open) closeOverlay(open);   // left from before a restart
  setShared(KEY_SCOPE, shared(KEY_SCOPE) ?? "agent");
  openPanel(placeNow());
}

// The empty window's title and line: no agent, the helper there cannot list them (`error`), or nothing yet.
export function emptyText(address, error = null) {
  const m = machineOf(address) ?? "this machine";
  if (!address) return ["No agent here", "Open an agent's tab to see what it pointed you to."];
  if (error === "old") return ["Helper too old", `The Flockbook helper on ${m} is too old to list resources. Update it to see them here.`];
  if (error === "missing") return ["No helper there", `${m} has no Flockbook helper yet. Install it from the chat to see resources here.`];
  return ["No resources yet", "Docs, artifacts and links the agent mentions in its replies show up here."];
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
    this.expanded = new Set(p.refit && Array.isArray(p.expanded) ? p.expanded : []); // groups shown past ROW_LIMIT
    cx?.spawn?.((cx) => (p.refit ? pollResources(cx) : pollResources(cx, true, true)));
    cx?.timer?.every?.(1000, (cx) => this.tick(cx));
    // A window does not take the keyboard when it opens: focus it, after the first frame, so Esc reaches it.
    if (!p.refit || p.focus) cx?.timer?.after?.(16, () => this.h?.focus?.());
  }

  tick(cx) {
    const a = currentAgent()?.address ?? null;
    // Another agent: its list from memory at once, then a full read (a doc it named may be there now; a delta from
    // its cursor would not say so when its log did not change).
    if (a !== this.address) {
      this.address = a; this.seenAtOpen = a ? shared(seenKey(term(a))) ?? null : null; this.note = null; this.expanded = new Set(); cx?.notify?.();
      if (a) cx?.spawn?.((cx) => pollResources(cx, true, true));
    }
    cx?.spawn?.((cx) => pollResources(cx));
    this.markSeen();
    this.refit();
  }

  // The item count, the shown agent or the window's size changed: open it again at its new size (no way to
  // resize an open window). At most once a tick.
  refit() {
    if (!panelOpen()) return;
    const p = placeNow([...this.expanded]);
    if (!needsRefit(shared(KEY_FRAME), p, this.inside || this.hover !== null)) return;
    openPanel(p, { refit: true, focus: Boolean(this.h?.is_focused?.()), seenAtOpen: this.seenAtOpen, expanded: [...this.expanded] }, props()?.overlay ?? shared(KEY_OPEN));
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
      // The count (owner 11-25: here, not in a footer, so the list has the room).
      .child(div().id("res-count").flex_none().text_size(11.5).text_color(t.muted)
        .child(`${items.length} item${items.length === 1 ? "" : "s"}${hidden ? ` · ${hidden} removed` : ""}`))
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
          .when(gone, (el) => el.opacity(0.5)).child(svg(icon(iconOf(it))).size(12).text_color(t.muted)))
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

    // A group's title folds it (a click anywhere on it); a long one draws ROW_LIMIT rows, then "Show N more".
    const title = (g) => h_flex().id(`res-group-head-${g.id}`).px(10).mx(4).pt(8).pb(3).gap(4).items_center().rounded(6)
      .hover((s) => s.text_color(t.foreground)).text_color(t.muted)
      .on_mouse_down("left", stop)
      .on_click((_e, cx) => { cx?.stop_propagation?.(); toggleGroup(g.id); cx?.notify?.(); })
      .child(svg(icon(g.collapsed ? "chevron-right" : "chevron-down")).id(`res-chev-${g.id}`).size(12).flex_none())
      .child(div().flex_1().text_size(11).font_weight(600).child(g.label))
      .child(div().text_size(11).font_weight(600).child(String(g.items.length)));
    const more = (g) => div().id(`res-more-${g.id}`).px(10).mx(4).py(6).rounded(8).text_size(12).text_color(t.muted)
      .hover((s) => s.bg(t.hover).text_color(t.foreground))
      .on_mouse_down("left", stop)
      .on_click((_e, cx) => { cx?.stop_propagation?.(); this.expanded.add(g.id); cx?.notify?.(); })
      .child(`Show ${g.rest} more`);
    const body = items.length
      ? v_flex().id("res-list").flex_1().min_h(0).py(4).overflow_y_scrollbar().children(layoutGroups(items, shared(KEY_COLLAPSED), [...this.expanded]).map((g) => v_flex().id(`res-group-${g.id}`)
        .child(title(g))
        .children(g.shown.map(row))
        .when(g.rest > 0, (el) => el.child(more(g)))))
      : v_flex().id("res-empty").flex_1().px(24).py(28).gap(6).items_center().justify_center()
        .child(div().text_size(13).font_weight(600).text_color(t.foreground).child(emptyText(address, d?.error)[0]))
        .child(div().text_size(12).text_color(t.muted).text_center().child(emptyText(address, d?.error)[1]));

    const root = v_flex().id("res-root").size_full().bg(t.background).border(1).border_color(t.border).shadow_lg()
      .when(props()?.drawer !== true, (el) => el.rounded(14))
      .on_hover((inside) => { this.inside = Boolean(inside); })
      // A press it does not stop drags the window: every press is stopped, so it stays where the chat made room.
      .on_mouse_down("left", (_e, cx) => { cx?.stop_propagation?.(); this.h?.focus?.(); })
      .on_key_down((e, cx) => { if (e?.key === "escape") { cx?.stop_propagation?.(); this.closeSelf(); } });
    if (this.h) root.track_focus(this.h);
    return root.child(header).child(body);
  }
}
