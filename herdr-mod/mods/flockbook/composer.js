// Under the chat: the status tag row (model, effort, context, cost, folder, branch, agent) and the
// box to reply in. Enter sends through the sidecar's `send`; "+" or a drop attaches files.
import { div, svg } from "gpui-kit";
import { v_flex, h_flex, Textarea, TextareaState } from "gpui-base";
import { theme, panes, home, exec } from "herdr";
import { agentName, icon, SPIN_MS } from "./state.js";
import * as sidecar from "./sidecar.js";

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

// The macOS file dialog, through osascript (a mod has no file picker of its own). exec stops after 30 s.
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

// ⌘V (no other modifier).
export function pasteKey(e) {
  const m = e?.modifiers ?? {};
  return e?.key === "v" && Boolean(m.platform || m.cmd) && !m.shift && !m.alt && !m.control;
}

export class Composer {
  // One input box, made here: herdr-app does not let a view make input state while it draws.
  // Each agent's unsent text and files are kept in `drafts` and swapped in when the agent changes.
  constructor(_cx) {
    this.input = TextareaState.new({ placeholder: "Reply to the agent", rows: 1 });
    this.input.set_auto_grow?.(1, 8);
    this.input.on("submit", (_e, cx) => { cx.spawn((cx) => this.submit(cx)); });
    this.empty = true;
    this.input.on("change", (_e, cx) => { const e = !this.input.value().trim(); if (e !== this.empty) { this.empty = e; cx.notify(); } });
    this.drafts = new Map();   // address → { text, files } of agents not shown
    this.address = null;
    this.files = [];
    this.sending = false;
    this.note = null;          // { text, bad } under the box, after a send
    this.status = new Map();   // address → status line
    this.statusAt = 0;
    this.statusBusy = false;
  }

  // Shows `agent`'s draft in the box. Safe while drawing: it only sets values.
  box(agent) {
    const a = agent?.address ?? null;
    if (a !== this.address) {
      if (this.address) this.drafts.set(this.address, { text: this.input.value(), files: this.files });
      const d = (a && this.drafts.get(a)) || { text: "", files: [] };
      this.drafts.delete(a);
      this.input.set_value(d.text); this.files = d.files; this.empty = !d.text.trim();
      this.address = a; this.note = null; this.statusAt = 0;
    }
    return a ? this : null;
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

  // ⌘V with no text on the clipboard: the sidecar saves the clipboard's image as a PNG, attached
  // like a picked file. Text pastes stay the box's own.
  async pasteImage(cx) {
    const r = await sidecar.pasteImage();
    const path = r.error ? null : r.lines.find((l) => l.type === "image")?.path;
    if (r.error) this.note = { text: r.error.message, bad: true };
    if (path) this.addFiles([path], cx);
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
    const text = b.input.value().replace(/\s+$/, "");
    if (!text.trim() && !b.files.length) { b.input.set_value(""); return; }
    const files = b.files;
    this.sending = true; this.note = null;
    b.input.set_value(""); b.files = [];
    cx.notify();
    try {
      const r = await sidecar.send(a, withFiles(text, files), files);
      const done = r.lines.find((l) => l.type === "done");
      if (r.error || !done || done.result === "failed") {
        // Put the text back, unless the box moved to another agent or something new was typed.
        if (this.address === a && !b.input.value()) { b.input.set_value(text); b.files = files; }
        else if (this.address !== a) this.drafts.set(a, { text, files });
        this.note = { text: (r.error?.message ?? done?.message) || "The message was not sent.", bad: true };
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
      .on_key_down((e, cx) => { if (pasteKey(e) && cx.read_from_clipboard?.() === undefined) cx.spawn((cx) => this.pasteImage(cx)); })
      .child(h_flex().id("tags").w_full().max_w(800).gap(5).items_center().overflow_x_scrollbar().children(tags.map(tag)))
      .child(v_flex().id("box").w_full().max_w(800).rounded(16).bg(t.raised).border(1).border_color(t.border).shadow_sm().px(12).pt(10).pb(8).gap(6)
        .when(this.files.length > 0, (el) => el.child(h_flex().gap(6).flex_wrap().children(this.files.map(fileChip))))
        .child(Textarea.new(b.input).w_full())
        .child(h_flex().gap(8).items_center()
          .child(div().id("attach").size(28).flex_none().flex().items_center().justify_center().rounded(14).text_color(t.muted)
            .hover((s) => s.bg(t.hover).text_color(t.foreground))
            .on_click((_e, cx) => cx.spawn((cx) => this.pick(cx))).child(svg(icon("plus")).size(15)))
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
