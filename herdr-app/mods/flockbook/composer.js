// Under the chat: the status tag row (model, effort, context, cost, folder, branch, agent) and the
// box to reply in. Enter sends through the sidecar's `send`; "+" or a drop attaches files.
import { div, svg } from "gpui-kit";
import { v_flex, h_flex, Textarea, TextareaState } from "gpui-base";
import { theme, panes, home, exec, window } from "herdr";
import { agentName, icon, SPIN_MS } from "./state.js";
import * as sidecar from "./sidecar.js";
import { setting } from "./config.js";

export const STATUS_MS = 5000;

// The pane of a terminal address: its folder and branch.
export function paneOf(address) {
  return panes().find((p) => p.terminal === address) ?? null;
}

// A folder's last part ("app"), or "~" for home: the full path is too long for a tag.
export function shortPath(path) {
  if (!path) return "";
  if (path === home()) return "~";
  return path.replace(/\/+$/, "").split("/").pop() || "/";
}

// The tags to show, in order: the sidecar's (model, effort, context, cost), then folder, branch, agent.
// The sidecar's model, effort, context and cost (its other tags repeat what is added here),
// then the folder, the branch and the agent. A text shown twice is shown once.
const FROM_SIDECAR = ["model", "effort", "context", "cost"];
export function tagsFor(agent, status) {
  const out = (status?.tags ?? []).filter((t) => FROM_SIDECAR.includes(t.kind))
    .map((t) => ({ kind: t.kind, text: t.text, warn: Boolean(t.warn) }));
  const pane = agent ? paneOf(agent.address) : null;
  if (pane?.cwd) out.push({ kind: "cwd", text: shortPath(pane.cwd), warn: false });
  if (pane?.branch) out.push({ kind: "branch", text: pane.branch, warn: false });
  if (agent) out.push({ kind: "agent", text: agentName(agent), warn: false });
  const seen = new Set();
  return out.filter((t) => t.text && !seen.has(t.text) && seen.add(t.text));
}

// The "+" button opens the macOS file dialog through osascript (a mod has no file picker of its own), so it
// shows on macOS only. Elsewhere, drop files on the chat or paste an image.
export const canPick = () => (window()?.platform ?? "macos") === "macos";

// The macOS file dialog, through osascript. exec stops after 30 s.
export async function chooseFiles() {
  const script = ['set l to choose file with prompt "Attach files" with multiple selections allowed',
    'set o to ""', "repeat with f in l", "set o to o & POSIX path of f & linefeed", "end repeat", "return o"];
  try {
    const r = await exec("osascript", script.flatMap((s) => ["-e", s]));
    if (r.code !== 0) return { files: [], error: /-128/.test(r.stderr) ? null : "The file dialog did not open. Drop the files on the chat instead." };
    return { files: r.stdout.split("\n").map((s) => s.trim()).filter(Boolean), error: null };
  } catch {
    return { files: [], error: "The file dialog closes after 30 seconds. Drop the files on the chat instead." };
  }
}

const base = (p) => p.split("/").pop();

// A path as the agent reads it: in single quotes when it has a space or a shell character.
export function quoted(path) {
  return /^[A-Za-z0-9_\/.,:@%+=~-]+$/.test(path) ? path : "'" + path.replace(/'/g, "'\\''") + "'";
}

// The text to send: the typed text, then each attached file's path. The sidecar's `send --file`
// expects the text to name every file (docs/protocol.md).
export function withFiles(text, files) {
  if (!files.length) return text;
  const paths = files.map(quoted).join(" ");
  return text ? `${text} ${paths}` : paths;
}

// ⌘V in the box (Ctrl+V on Linux). The box's own paste runs before any key handler and types the clipboard's
// text, so a file copied in Finder came in as its name. The mod binds the chord under its own key context
// instead (the keymap is the app's; the context keeps it to this box) and asks the sidecar first.
export const PASTE_ACTION = "flockbook_paste";
export const PASTE_CONTEXT = "FlockbookComposer";
let pasteBound = false;
function bindPaste(cx) {
  if (pasteBound || typeof cx?.bind_keys !== "function") return;
  try {
    cx.bind_keys([{ keystroke: "secondary-v", action: PASTE_ACTION, context: `${PASTE_CONTEXT} > Input` }]);
    pasteBound = true;
  } catch { /* the box's own paste stays, and paste_other still attaches an image */ }
}

// "/" commands (docs/protocol.md, commands; the helper lists them for Claude Code, Codex, pi and agy, and none for
// other agents): while the text is "/" and a name with no space, a
// list of the agent's commands shows above the box; after "/model " (a built-in with values) its values. All the
// matches are in the list; SLASH_ROWS show at a time, and the window follows the pick (↑ ↓ stop at the ends, no
// wrap) or the wheel. Tab or Enter completes, Esc closes.
// The box takes ↑ ↓ and Tab itself (as ⌘V), so they are bound under a key context the box's wrapper has only while
// the list shows: without the list they move the cursor as before. Only full names go out: Claude Code's own "/"
// menu takes the Enter of a part of a name and runs its first match.
export const SLASH_CONTEXT = "FlockbookSlash";
export const SLASH = { up: "flockbook_slash_up", down: "flockbook_slash_down", complete: "flockbook_slash_complete", close: "flockbook_slash_close" };
export const SLASH_ROWS = 8;
const CMDS_MS = 30000;   // the list is asked again after this long
let slashBound = false;
function bindSlash(cx) {
  if (slashBound || typeof cx?.bind_keys !== "function") return;
  try {
    cx.bind_keys([["up", SLASH.up], ["down", SLASH.down], ["tab", SLASH.complete], ["escape", SLASH.close]]
      .map(([keystroke, action]) => ({ keystroke, action, context: `${SLASH_CONTEXT} > Input` })));
    slashBound = true;
  } catch { /* no keys for the list: a click on a row and Enter still complete */ }
}

// The name being typed after a leading "/", or null: the text has a space (or is not a "/" name).
export function slashQuery(text) {
  const m = /^\/([^\s]*)$/.exec(String(text ?? ""));
  return m ? m[1] : null;
}

// The commands whose name starts with `query` (any case), in the list's order. All of them: the list scrolls.
export function matchCommands(list, query) {
  const q = String(query ?? "").toLowerCase();
  return (list ?? []).filter((c) => String(c.name).toLowerCase().startsWith(q));
}

// "/<name> <part>" (one space, no other): the command and the part of a value typed after it; else null.
export function argQuery(text) {
  const m = /^\/([^\s]+) ([^\s]*)$/.exec(String(text ?? ""));
  return m ? { name: m[1], query: m[2] } : null;
}

// The first shown row so that `pick` is shown, from the window's last top. Keeps the window still while it can.
export function windowTop(top, pick, n, rows = SLASH_ROWS) {
  let t = Math.max(0, Math.min(top, Math.max(0, n - rows)));
  if (pick < t) t = pick;
  else if (pick >= t + rows) t = pick - rows + 1;
  return t;
}
const ROW_PX = 30;   // a list row; the wheel moves the window a row per ROW_PX


// What `paste-image` found to attach: copied files, else the saved image. Empty: paste the text.
export function pastedFiles(lines) {
  const files = lines.find((l) => l.type === "files")?.paths;
  if (Array.isArray(files) && files.some(Boolean)) return files.filter((p) => typeof p === "string" && p);
  const image = lines.find((l) => l.type === "image")?.path;
  return image ? [image] : [];
}

// Enter in the box puts a new line at the cursor (over a selection) before it reports the submit, so in the
// middle of the text that line went out. `before` is the text just before the key: when `now` is `before` with
// one span turned into "\n", the text to send is `before`. Anything else is sent as it is.
export function withoutEnter(now, before) {
  if (typeof before !== "string" || now === before) return now;
  let i = 0;
  while (i < now.length && i < before.length && now[i] === before[i]) i++;
  let j = 0;
  while (j < now.length - i && j < before.length - i && now[now.length - 1 - j] === before[before.length - 1 - j]) j++;
  return now.slice(i, now.length - j) === "\n" ? before : now;
}

export class Composer {
  // One input box, made here: herdr-app does not let a view make input state while it draws.
  // Each agent's unsent text and files are kept in `drafts` and swapped in when the agent changes.
  constructor(_cx) {
    this.input = TextareaState.new({ placeholder: "Reply to the agent", rows: 1 });
    this.input.set_auto_grow?.(1, 8);
    bindPaste(_cx);
    bindSlash(_cx);
    // A paste that found no text, when the binding above is missing: the clipboard's image, if any.
    this.input.on("paste_other", (_e, cx) => cx.spawn((cx) => this.paste(cx, undefined)));
    // Enter sends. On Shift+Enter the box puts the new line in itself (at the cursor, over a selection);
    // the submit with shift only reports it, so do nothing here. Adding "\n" here gives two new lines.
    this.input.on("submit", (e, cx) => {
      if (e?.shift) return;
      cx.spawn((cx) => this.submit(cx));
    });
    this.empty = true;
    // The last two texts the box reported, so a submit knows the text before Enter's new line (the box
    // reports that line's change before or after the submit).
    this.typed = ["", ""];
    this.input.on("change", (_e, cx) => {
      const v = this.input.value();
      if (v !== this.typed[1]) this.typed = [this.typed[1], v];
      const e = !v.trim(); if (e !== this.empty) { this.empty = e; cx.notify(); }
      if (this.slashClosed !== null && v !== this.slashClosed) this.slashClosed = null;
      this.pick = 0; this.top = 0; this.wheel = 0;
      if (slashQuery(v) !== null || argQuery(v) !== null) cx.spawn((cx) => this.loadCommands(cx));
      cx.notify();
    });
    this.cmds = new Map();      // address → { at, list } of a Claude Code agent's "/" commands
    this.cmdsBusy = false;
    this.agent = null;          // the agent the box is for (its kind decides the list)
    this.pick = 0;              // the chosen match of the list
    this.top = 0;               // the first shown match
    this.wheel = 0;             // wheel pixels not yet a whole row
    this.slashClosed = null;    // the text Esc closed the list at
    this.drafts = new Map();   // address → { text, files } of agents not shown
    this.address = null;
    // address → { agent, path } the chat showed last (set by the chat); send is bound to it.
    this.seen = null;
    this.files = [];
    this.sending = false;
    this.note = null;          // { text, bad } under the box, after a send
    this.status = new Map();   // address → status line
    this.statusAt = 0;
    this.statusBusy = false;
  }

  // Sets the box's text from here (no change report counts on it).
  setText(v) { this.input.set_value(v); this.typed = [v, v]; }

  // Shows `agent`'s draft in the box. Safe while drawing: it only sets values.
  box(agent) {
    this.agent = agent ?? null;
    const a = agent?.address ?? null;
    if (a !== this.address) {
      if (this.address) this.drafts.set(this.address, { text: this.input.value(), files: this.files });
      const d = (a && this.drafts.get(a)) || { text: "", files: [] };
      this.drafts.delete(a);
      this.setText(d.text); this.files = d.files; this.empty = !d.text.trim();
      this.address = a; this.note = null; this.statusAt = 0;
    }
    return a ? this : null;
  }

  // Asks the helper for the shown agent's commands (at most every CMDS_MS), if it has `commands`. An agent it lists
  // none for gets an empty list: no list shows.
  async loadCommands(cx) {
    const a = this.address;
    if (!a || !this.agent || this.cmdsBusy) return;
    const had = this.cmds.get(a);
    if (had && Date.now() - had.at < CMDS_MS) return;
    this.cmdsBusy = true;
    try {
      if (!(await sidecar.hasCommand("commands"))) return;
      const r = await sidecar.commands(a);
      const list = r.error ? had?.list ?? [] : (r.lines.find((l) => l.type === "commands")?.commands ?? []);
      this.cmds.set(a, { at: Date.now(), list });
      cx.notify();
    } finally { this.cmdsBusy = false; }
  }

  // The list to show now: { kind, query, items, pick, top, cmd } or null. `kind` "cmd": commands for "/<part>";
  // "arg": the values of `cmd` for "/<cmd> <part>" (items are { name: value }).
  slash() {
    const v = this.input.value();
    return v === this.slashClosed ? null : this.slashFor(v);
  }

  slashFor(v) {
    if (!this.agent || !this.address) return null;
    const list = this.cmds.get(this.address)?.list ?? [];
    let kind = "cmd", query = slashQuery(v), items, cmd = null;
    if (query !== null) items = matchCommands(list, query);
    else {
      const a = argQuery(v);
      cmd = a && list.find((c) => c.name === a.name && Array.isArray(c.args) && c.args.length);
      if (!cmd) return null;
      kind = "arg"; query = a.query;
      items = matchCommands(cmd.args.map((x) => ({ name: String(x) })), query);
    }
    if (!items.length) return null;
    const pick = Math.min(this.pick, items.length - 1);
    return { kind, query, items, pick, top: windowTop(this.top, pick, items.length), cmd };
  }

  // A command: "/<name> " (whole, and the space closes the list or opens its values). A value: "/<cmd> <value>".
  complete(s, item, cx) {
    this.setText(s.kind === "arg" ? `/${s.cmd.name} ${item.name}` : `/${item.name} `);
    this.empty = false; this.pick = 0; this.top = 0; this.wheel = 0; this.slashClosed = null;
    cx?.notify?.();
  }

  slashKey(what, cx) {
    const s = this.slash();
    if (!s) return;
    if (what === "up") this.pick = Math.max(0, s.pick - 1);
    else if (what === "down") this.pick = Math.min(s.items.length - 1, s.pick + 1);
    else if (what === "complete") return this.complete(s, s.items[s.pick], cx);
    else if (what === "close") this.slashClosed = this.input.value();
    this.top = windowTop(s.top, this.pick, s.items.length);
    cx.notify();
  }

  // Wheel or trackpad over the list: the window moves a row per ROW_PX (small steps add up), and the pick stays on
  // a shown row.
  slashWheel(e, cx) {
    const s = this.slash();
    if (!s || s.items.length <= SLASH_ROWS) return;
    this.wheel -= Number(e?.delta?.y) || 0;   // y > 0: up
    const rows = Math.trunc(this.wheel / ROW_PX);
    if (!rows) return;
    this.wheel -= rows * ROW_PX;
    this.top = Math.max(0, Math.min(s.items.length - SLASH_ROWS, s.top + rows));
    this.pick = Math.max(this.top, Math.min(this.top + SLASH_ROWS - 1, s.pick));
    cx.notify();
  }

  async pollStatus(cx) {
    const a = this.address;
    if (!a || this.statusBusy || Date.now() - this.statusAt < STATUS_MS) return;
    this.statusBusy = true; this.statusAt = Date.now();
    try {
      const r = await sidecar.status(a);
      const line = r.error ? null : r.lines.find((l) => l.type === "status") ?? null;
      const before = JSON.stringify(this.status.get(a) ?? null);
      this.status.set(a, line);
      if (JSON.stringify(line) !== before) cx.notify();
    } finally { this.statusBusy = false; }
  }

  addFiles(paths, cx) {
    const add = [...new Set(paths || [])].filter((p) => p && !this.files.includes(p));
    if (!add.length) return;
    this.files = this.files.concat(add); this.note = null; cx.notify();
  }

  // ⌘V: copied files (Finder) are attached like picked files; plain text is typed at the cursor, also with a
  // picture beside it (spreadsheet cells); an image alone (a screenshot, a browser's Copy Image) is attached.
  // The sidecar decides (paste-image). `text` is the clipboard's text when the key was pressed. A broken
  // sidecar still lets text paste.
  async paste(cx, text) {
    const a = this.address;
    const r = await sidecar.pasteImage();
    if (this.address !== a) return;
    const files = r.error ? [] : pastedFiles(r.lines);
    if (files.length) return this.addFiles(files, cx);
    if (text) { this.input.replace(text); const v = this.input.value(); this.typed = [v, v]; this.empty = !v.trim(); }
    else if (r.error) this.note = { text: r.error.message, bad: true };
    cx.notify();
  }

  async pick(cx) {
    const r = await chooseFiles();
    if (r.error) this.note = { text: r.error, bad: true };
    this.addFiles(r.files, cx);
    cx.notify();
  }

  async submit(cx) {
    const a = this.address, b = this;
    if (this.sending || !a) return;
    const now = b.input.value();
    const text = withoutEnter(now, now === this.typed[1] ? this.typed[0] : this.typed[1]).replace(/\s+$/, "");
    if (!text.trim() && !b.files.length) { b.setText(""); return; }
    // The list shows and the name is not whole yet: Enter completes it (a part of a name would run another command).
    const s = text !== this.slashClosed ? this.slashFor(text) : null;
    if (s && !s.items.some((c) => c.name === s.query)) return this.complete(s, s.items[s.pick], cx);
    const files = b.files;
    this.sending = true; this.note = null;
    b.setText(""); b.files = [];
    cx.notify();
    try {
      const r = await sidecar.send(a, withFiles(text, files), files, this.seen?.(a) ?? null);
      const done = r.lines.find((l) => l.type === "done");
      const moved = sidecar.agentChanged(done);
      // Only "sent" and "unconfirmed" (it may still arrive) mean the text went out; any other result did not send.
      if (r.error || !done || (done.result !== "sent" && done.result !== "unconfirmed")) {
        // Put the text back, unless the box moved to another agent or something new was typed.
        if (this.address === a && !b.input.value()) { b.setText(text); b.files = files; }
        else if (this.address !== a) this.drafts.set(a, { text, files });
        this.note = { text: moved ? sidecar.AGENT_CHANGED : (r.error?.message ?? done?.message) || "The message was not sent.", bad: true };
      } else if (done.result === "unconfirmed") {
        this.note = { text: "Sent. The agent did not confirm it yet.", bad: false };
      }
    } finally { this.sending = false; cx.notify(); }
  }

  render(agent, t) {
    const tags = tagsFor(agent, agent ? this.status.get(agent.address) : null);
    const branch = tags.find((x) => x.kind === "branch");
    const b = this.box(agent);
    if (!b) return div();
    const s = this.slash();
    const shown = s ? s.items.slice(s.top, s.top + SLASH_ROWS) : [];
    const row = (c, j) => h_flex().id(`slash-${s.top + j}`).w_full().h(ROW_PX).px(10).gap(10).items_center().rounded(8)
      .when(s.top + j === s.pick, (el) => el.bg(t.hover))
      .hover((st) => st.bg(t.hover))
      .on_click((_e, cx) => this.complete(s, c, cx))
      .child(div().flex_none().text_size(13).font_semibold().text_color(t.foreground).child(s.kind === "arg" ? c.name : `/${c.name}`))
      .when(Boolean(c.hint), (el) => el.child(div().flex_none().text_size(12).text_color(t.dim).child(c.hint)))
      .child(div().flex_1().min_w(0).truncate().text_size(12).text_color(t.muted).child(c.description || ""))
      // Sent bare, it opens a screen in the terminal (a picker, settings): answer it there.
      .when(Boolean(c.screen), (el) => el.child(div().flex_none().text_size(11).text_color(t.dim).child("opens a screen")));
    const more = s && s.items.length > SLASH_ROWS
      ? div().id("slash-more").w_full().px(10).pt(2).text_size(11).text_color(t.dim)
        .child(`${s.top + 1}–${s.top + shown.length} of ${s.items.length}`)
      : null;
    const empty = !b.input.value().trim() && !b.files.length;
    const tag = (x) => h_flex().h(22).px(8).gap(4).items_center().rounded(11).flex_none().text_size(11.5)
      .bg(x.warn ? t.blocked : t.chrome).text_color(x.warn ? t.background : t.muted).border(1).border_color(x.warn ? t.blocked : t.border)
      .when(x.kind === "branch", (el) => el.child(svg(icon("branch")).size(11)))
      .when(x.kind === "cwd", (el) => el.child(svg(icon("folder")).size(11)))
      .child(div().max_w(220).truncate().child(x.text));
    const fileChip = (p, i) => h_flex().id(`file-${i}`).h(24).pl(8).pr(4).gap(4).items_center().rounded(8).bg(t.chrome).text_size(12).text_color(t.foreground)
      .child(div().max_w(180).truncate().child(base(p)))
      .child(div().id(`file-x-${i}`).size(16).flex().items_center().justify_center().rounded(4).text_color(t.muted).hover((s) => s.bg(t.hover))
        .on_click((_e, cx) => { this.files = this.files.filter((f) => f !== p); cx.notify(); }).child("×"));
    return v_flex().id("composer").w_full().px(16).pb(14).pt(6).gap(6).items_center()
      .key_context(PASTE_CONTEXT)
      .on_action(PASTE_ACTION, (_e, cx) => { const text = cx.read_from_clipboard?.(); cx.spawn((cx) => this.paste(cx, text)); })
      .when(setting("status_row") !== "hide", (el) => el.child(h_flex().id("tags").w_full().max_w(800).gap(5).items_center().overflow_x_scrollbar().children(tags.map(tag))))
      .when(Boolean(s), (el) => el.child(v_flex().id("slash").w_full().max_w(800).p(4).gap(0).rounded(12).bg(t.raised)
        .border(1).border_color(t.border).shadow_sm().on_scroll_wheel((e, cx) => this.slashWheel(e, cx))
        .children(shown.map(row)).when(Boolean(more), (l) => l.child(more))))
      .child(v_flex().id("box").w_full().max_w(800).rounded(16).bg(t.raised).border(1).border_color(t.border).shadow_sm().px(12).pt(10).pb(8).gap(6)
        .when(this.files.length > 0, (el) => el.child(h_flex().gap(6).flex_wrap().children(this.files.map(fileChip))))
        .child(div().id("slash-keys").w_full()
          .when(Boolean(s), (el) => el.key_context(SLASH_CONTEXT))
          .on_action(SLASH.up, (_e, cx) => this.slashKey("up", cx))
          .on_action(SLASH.down, (_e, cx) => this.slashKey("down", cx))
          .on_action(SLASH.complete, (_e, cx) => this.slashKey("complete", cx))
          .on_action(SLASH.close, (_e, cx) => this.slashKey("close", cx))
          .child(Textarea.new(b.input).w_full()))
        .child(h_flex().gap(8).items_center()
          .when(canPick(), (el) => el.child(div().id("attach").size(28).flex_none().flex().items_center().justify_center().rounded(14).text_color(t.muted)
            .hover((s) => s.bg(t.hover).text_color(t.foreground))
            .on_click((_e, cx) => cx.spawn((cx) => this.pick(cx))).child(svg(icon("plus")).size(15))))
          .child(div().flex_1())
          .when(Boolean(branch), (el) => el.child(h_flex().gap(4).items_center().text_size(12).text_color(t.muted)
            .child(svg(icon("branch")).size(12)).child(div().max_w(200).truncate().child(branch.text))))
          .child(div().id("send").size(28).flex_none().flex().items_center().justify_center().rounded(14)
            .bg(empty || this.sending ? t.chrome : t.accent).text_color(empty || this.sending ? t.dim : t.background)
            .on_click((_e, cx) => cx.spawn((cx) => this.submit(cx)))
            .child(this.sending ? svg(icon("spinner")).size(13).spin(SPIN_MS) : svg(icon("send")).size(14)))))
      .when(Boolean(this.note), (el) => el.child(div().id("send-note").w_full().max_w(800).text_size(12)
        .text_color(this.note.bad ? t.blocked : t.muted).child(this.note.text)));
  }
}
