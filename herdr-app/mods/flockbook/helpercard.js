// Terminals on another machine (`m/t7`, Rust sidecar only): the chat runs the Flockbook helper on
// that machine. Putting a program on another machine, or taking it off, needs a press: a card says
// which machine, where, how big and its checksum. Nothing here runs on a timer.
import { div } from "gpui-kit";
import { v_flex, h_flex } from "gpui-base";
import { shared, setShared } from "herdr";
import { KEY_REMOVE } from "./state.js";
import * as sidecar from "./sidecar.js";

const REMOTE = new Set(["remote_helper_missing", "remote_helper_outdated", "remote_helper_unsupported", "remote_failed", "bad_terminal"]);
export const isRemoteProblem = (err) => REMOTE.has(err?.code);
export const mb = (bytes) => `${(bytes / 1e6).toFixed(1)} MB`;

export class HelperUI {
  constructor() {
    this.state = new Map();   // machine → { step: "busy" | "failed" | "cancelled", message }
    this.removing = null;     // { machine, step: "busy" | "failed" | "done", message }
    this.asked = null;        // the KEY_REMOVE value last drawn
  }

  // KEY_REMOVE is set by the tab menu (another copy of this mod). True when the view must redraw.
  changed() {
    const m = shared(KEY_REMOVE) ?? null;
    if (m === this.asked) return false;
    this.asked = m;
    if (m) this.removing = null;
    return true;
  }

  button(t, id, label, kind, action) {
    const primary = kind === "primary", danger = kind === "danger";
    return div().id(id).h(30).px(14).flex().items_center().rounded(8).text_size(13)
      .bg(primary ? t.accent : danger ? t.blocked : t.chrome).text_color(primary || danger ? t.background : t.foreground)
      .border(1).border_color(primary ? t.accent : danger ? t.blocked : t.border)
      .hover((s) => s.opacity(0.9)).on_click((_e, cx) => action(cx))
      .child(label);
  }

  box(t) { return v_flex().w_full().max_w(620).p(16).gap(10).rounded(14).bg(t.raised); }

  // The words (and the install card) for a remote error. `refresh(cx)` polls the chat again.
  problem(t, err, at, refresh) {
    const machine = err.install?.machine ?? at ?? "that machine";
    const small = (text) => div().max_w(560).text_size(11.5).text_color(t.dim).child(text || "");
    const wrap = (...kids) => v_flex().size_full().items_center().justify_center().gap(8).px(32).children(kids.filter(Boolean));
    const say = (text) => div().text_size(13.5).text_color(t.foreground).child(text);
    switch (err.code) {
      case "remote_helper_unsupported": return wrap(say(`The Flockbook helper cannot run on ${machine} yet (Linux and macOS only).`), small(err.message));
      case "remote_failed": return wrap(say("The other machine did not answer. Trying again."), small(err.message));
      case "bad_terminal": return wrap(say("This is not a terminal address Flockbook can read."), small(err.message));
    }
    const other = err.code === "remote_helper_outdated" ? err.remote?.sidecar : null;
    const why = other ? `The helper on ${machine} is another version (${other}).` : `${machine} has no Flockbook helper yet, so the chat cannot read its agents.`;
    if (!err.install) return wrap(say(why), say("This Flockbook has no helper for that machine. Install the Flockbook release that includes it."), small(err.message));
    const i = err.install, s = this.state.get(i.machine);
    if (s?.step === "cancelled") return wrap(say(why), small("Not installed. The chat for this machine stays off."),
      this.button(t, "helper-again", "Install…", "plain", (cx) => { this.state.delete(i.machine); cx.notify(); }));
    const row = (k, v) => h_flex().gap(10).items_start().text_size(12)
      .child(div().w(76).flex_none().text_color(t.muted).child(k))
      .child(div().flex_1().min_w(0).text_color(t.foreground).child(v));
    const card = this.box(t).id("helper-card")
      .child(div().text_size(13.5).font_semibold().text_color(t.foreground).child(`Install the Flockbook helper on ${i.machine}?`))
      .child(div().text_size(12).text_color(t.muted).child(other ? `${why} This puts the version of this Flockbook beside it.` : why))
      .when(i.source === "dev", (el) => el.child(div().id("helper-dev").px(8).py(4).rounded(6).bg(t.blocked).text_color(t.background)
        .text_size(12).font_semibold().child("Development source (FLOCKBOOK_REMOTE_HELPERS), not a release file")))
      .child(row("Machine", i.machine))
      .child(row("Path", i.path))
      .child(row("Size", mb(i.bytes)))
      .child(row("Platform", i.target))
      .child(row("SHA-256", i.sha256));
    if (s?.step === "busy") card.child(div().text_size(12).text_color(t.muted).child(`Installing on ${i.machine}…`));
    else {
      if (s?.step === "failed") card.child(div().id("helper-note").text_size(12).text_color(t.blocked).child(`Nothing was installed. ${s.message}`));
      card.child(h_flex().gap(8).justify_end()
        .child(this.button(t, "helper-cancel", "Cancel", "plain", (cx) => { this.state.set(i.machine, { step: "cancelled" }); cx.notify(); }))
        .child(this.button(t, "helper-install", "Install", "primary", (cx) => this.install(cx, i, refresh))));
    }
    return wrap(card);
  }

  install(cx, i, refresh) {
    this.state.set(i.machine, { step: "busy" }); cx.notify();
    return cx.spawn(async (cx) => {
      const r = await sidecar.installHelper(i.machine, i.sha256);
      const done = r.lines.find((l) => l.type === "installed");
      if (r.error || !done) this.state.set(i.machine, { step: "failed", message: r.error?.message ?? "The helper did not say it was installed." });
      else { this.state.delete(i.machine); await refresh(cx); }
      cx.notify();
    });
  }

  // The remove card, when the tab menu asked for it; else null.
  removeCard(t, refresh) {
    const m = this.asked;
    const r = this.removing;
    if (!m && !r) return null;
    const card = this.box(t).id("helper-remove-card");
    if (r?.step === "done") return card.child(div().text_size(12.5).text_color(t.foreground).child(`Removed the Flockbook helper from ${r.machine}.`))
      .child(h_flex().justify_end().child(this.button(t, "helper-ok", "OK", "plain", (cx) => { this.removing = null; cx.notify(); })));
    const machine = m ?? r.machine;
    card.child(div().text_size(13.5).font_semibold().text_color(t.foreground).child(`Remove the Flockbook helper from ${machine}?`))
      .child(div().text_size(12).text_color(t.muted).child(`This deletes ~/.local/share/flockbook/helper on ${machine}. The chat for its agents stops until you install it again.`));
    if (r?.step === "busy") return card.child(div().text_size(12).text_color(t.muted).child(`Removing from ${machine}…`));
    if (r?.step === "failed") card.child(div().text_size(12).text_color(t.blocked).child(r.message));
    return card.child(h_flex().gap(8).justify_end()
      .child(this.button(t, "helper-keep", "Keep", "plain", (cx) => { this.removing = null; setShared(KEY_REMOVE, null); this.asked = null; cx.notify(); }))
      .child(this.button(t, "helper-remove", "Remove", "danger", (cx) => this.remove(cx, machine, refresh))));
  }

  remove(cx, machine, refresh) {
    this.removing = { machine, step: "busy" }; cx.notify();
    return cx.spawn(async (cx) => {
      const r = await sidecar.removeHelper(machine);
      setShared(KEY_REMOVE, null); this.asked = null;
      this.removing = r.error ? { machine, step: "failed", message: r.error.message } : { machine, step: "done" };
      await refresh(cx);
      cx.notify();
    });
  }
}
