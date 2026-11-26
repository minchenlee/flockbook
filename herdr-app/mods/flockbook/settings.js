// The "settings" view: the Flockbook settings dialog (sidebar row "Flockbook", ⌘⇧, or the tab menu),
// over the window as Herdr's own Settings, and laid out like it: sections in a sidebar on the left,
// UPPERCASE headings on the right, a row's title and line on the left and its control on the right.
// Esc, a click beside it or its close button closes it. Settings of a few choices and one line of text, saved in
// <appHome>/flockbook/config.json (config.js), and the Phone page (phone.js). Tab moves through the
// sections, then the choices; Enter or Space picks one. Also the update line, which the sidebar shows too.
import { div, svg } from "gpui-kit";
import { v_flex, h_flex, Input, InputState } from "gpui-base";
import { theme, props, closeOverlay, shared, setShared } from "herdr";
import * as sidecar from "./sidecar.js";
import { setting, setSetting, loadConfig, configBad, configPath, updateShown, dismissUpdate } from "./config.js";
import { icon, settingsClosed } from "./state.js";
import { pollPhone, phoneTick, PHONE_TICK_MS, phoneOn, phoneBlocked, phoneChoose, phoneUser, phoneBody, setPhoneOption } from "./phone.js";

const PHONE = { title: "Phone page", about: "Read your agents on your phone, through Tailscale." };

export const SETTINGS = [
  { key: "check_updates", title: "Check for updates", choices: [[true, "On", "on"], [false, "Off", "off"]],
    about: "Once a day, Flockbook asks GitHub (api.github.com) for its latest release and shows a line when a newer one is out." },
  { key: "text_size", title: "Text size", choices: [["small", "Small", "small"], ["default", "Default", "default"], ["large", "Large", "large"]],
    about: "The size of the messages, code and tables in the chat. ⌥⌘- and ⌥⌘= step it, ⌥⌘0 resets it; Herdr's zoom (⌘- and ⌘=) scales the whole window." },
  { key: "status_row", title: "Status row", choices: [["show", "Show", "show"], ["hide", "Hide", "hide"]],
    about: "The model, context, cost, folder and branch above the reply box." },
  { key: "remote_helper", title: "Helper on other machines", choices: [["ask", "Ask", "ask"], ["never", "Never", "never"]],
    about: "For a tab on another machine, the chat asks before it installs the Flockbook helper there. Never: it does not ask, and nothing runs there. To remove a helper that is already there, choose Ask first." },
  { key: "summaries", title: "Summaries", choices: [["off", "Off", "off"], ["on", "On", "on"]],
    about: "A model writes two lines about an agent's last turn, for the chat's Since you left card. Off: nothing is sent to a model." },
  // `notes`: a line for the chosen one, under the choices. The row is stacked: its four choices are wide.
  { key: "summary_by", title: "Written by", choices: [["own", "Each agent's own", "own"], ["claude", "Claude Code", "claude"], ["codex", "Codex", "codex"], ["pi", "pi", "pi"]],
    about: "The program that writes the summaries, on its own account.",
    notes: {
      own: "Claude Code, Codex and pi agents each use their own program. A turn goes only where that conversation already goes. Other agents get no summary.",
      claude: "Claude Code writes for every agent. The turns of other agents go to Claude Code's provider too.",
      codex: "Codex writes for every agent. The turns of other agents go to Codex's provider too.",
      pi: "pi writes for every agent. The turns of other agents go to pi's provider too.",
    } },
];
// Summaries on: what is sent, under the row.
const SUMMARY_SENT = "When an agent stops, the end of its last turn and your message that started it go to the model. A model can be wrong; the line from the log stays on the card.";
// The model: a line of text with the program's models under it, not choices. `empty`: what runs when no model is named.
const MODEL = { key: "summary_model", title: "Model", about: "Pick one from the list, or type a name." };
const MODEL_EMPTY = { claude: "claude-haiku-5-5", codex: "the model Codex uses by default", pi: "the model pi uses by default" };
const MODEL_SOURCE = {
  "built-in": "Claude Code has no command that lists its models. This is a short list kept in Flockbook; you can type another name.",
  file: "From Codex's own list on this computer.",
  program: "From pi --list-models.",
};
// "Today: 42 runs of a model, about 96,000 characters sent and 5,100 back.\nLast 7 days: 210 runs, …" from the
// helper's count, a line for each time; "" when there is none. Characters, not tokens: the programs do not all say their tokens.
const about = (n) => (n >= 10000 ? Math.round(n / 1000) * 1000 : n >= 1000 ? Math.round(n / 100) * 100 : n).toLocaleString("en-US");
export function usageText(u) {
  if (!u?.today || !u?.week) return "";
  const part = (x) => `${x.runs} ${x.runs === 1 ? "run" : "runs"} of a model${x.failed ? ` (${x.failed} failed)` : ""}, about ${about(x.sent)} characters sent and ${about(x.got)} back`;
  if (!u.week.runs) return "No model ran for a summary in the last 7 days.";
  return `Today: ${part(u.today)}.\nLast 7 days: ${part(u.week)}.`;
}
const KEY_USAGE = "flockbook.summary.usage";     // { today, week }: the helper's summary-usage lines, or null
const KEY_MODELS = "flockbook.summary.models";   // { [program]: { models: [{ id, note }], source } | { error } }
const MODELS_SHOWN = 8;
// The models to show for what the field holds: all of them while it holds the saved name (or nothing), else those
// with the typed text in their name.
export function modelMatches(models, typed, saved) {
  const q = String(typed ?? "").trim().toLowerCase();
  return !q || q === String(saved ?? "").toLowerCase() ? models : models.filter((m) => m.id.toLowerCase().includes(q));
}

// The sidebar's sections; `parts`: [heading, setting keys] in order.
export const SECTIONS = [
  { id: "general", label: "General", icon: "sliders", parts: [["CHAT", ["text_size", "status_row"]], ["WHAT AGENTS DID", ["summaries", "summary_by", "summary_model"]], ["OTHER MACHINES", ["remote_helper"]]] },
  { id: "phone", label: "Phone page", icon: "phone" },
  { id: "updates", label: "Updates", icon: "refresh", parts: [["UPDATES", ["check_updates"]]] },
];

const picks = (e) => e?.key === "enter" || e?.key === "space" || e?.key === " ";

// "Flockbook X is out" with Copy (the one-line install) and Not now. Null when there is nothing to say.
// `full`: the command is written out too (the settings dialog; the sidebar is narrow).
const smallButton = (t) => (id, label, act) => div().id(id).h(24).px(9).flex().items_center().rounded(6).text_size(12)
  .text_color(t.foreground).bg(t.chrome).border(1).border_color(t.border).hover((s) => s.bg(t.hover))
  .on_mouse_down("left", (_e, cx) => cx.stop_propagation()).on_click((_e, cx) => act(cx)).child(label);

export function updateLine(t, owner, full = false) {
  const u = updateShown();
  if (!u) return null;
  const small = smallButton(t);
  const copy = (cx) => { cx.write_to_clipboard?.(u.command); owner.copied = u.latest; cx.notify?.(); };
  return v_flex().id("update-line").w_full().px(10).py(8).gap(6).rounded(8).bg(t.raised).border(1).border_color(t.border)
    .child(div().text_size(12.5).text_color(t.foreground).child(`Flockbook ${u.latest} is out`))
    .when(full, (el) => el.child(div().id("update-command").text_size(11.5).font_family("Menlo").text_color(t.muted).child(u.command)))
    .child(h_flex().gap(6).items_center()
      .child(small("update-copy", owner.copied === u.latest ? "Copied" : "Copy install command", copy))
      .child(small("update-dismiss", "Not now", (cx) => cx.spawn(async (cx) => { await dismissUpdate(); cx.notify(); }))));
}

export class SettingsView {
  constructor(cx) {
    this.copied = null;
    this.section = "general";
    this.overlay = props()?.overlay ?? null;
    // The Phone page's login: saved on Enter or when the field loses the keys.
    this.login = InputState.new({ placeholder: "Default: this machine's Tailscale login" });
    const save = (_e, cx) => {
      const v = this.login.value().trim();
      if (v === phoneUser()) return;
      cx.spawn(async (cx) => { await setPhoneOption("phone_user", v); cx.notify(); });
    };
    this.login.on("submit", save);
    this.login.on("blur", save);
    // The summary's model: saved the same way.
    this.model = InputState.new({ placeholder: "Default" });
    const saveModel = (_e, cx) => this.pickModel(this.model.value(), cx);
    this.model.on("submit", saveModel);
    this.model.on("blur", saveModel);
    // The list under the field follows what is typed.
    this.model.on("change", (_e, cx) => cx.notify?.());
    cx?.spawn?.(async (cx) => { await loadConfig(); this.login.set_value(phoneUser()); this.model.set_value(setting("summary_model")); cx.notify(); await this.loadModels(cx); await this.loadUsage(cx); });
    cx?.spawn?.(async (cx) => { await pollPhone(false); cx.notify(); });
    // Every 10 s; every 3 s while On waits for Serve (phone.js).
    cx?.timer?.every(PHONE_TICK_MS, (cx) => cx.spawn(async (cx) => { await phoneTick(); cx.notify(); }));
  }

  close(_cx) { settingsClosed(this.overlay); }

  loginInput() { return Input.new(this.login).w_full(); }
  modelInput() { return Input.new(this.model).w_full(); }

  // Another program: the model named for the one before is not its model, so the field is emptied.
  choose(key, value, cx) {
    cx.spawn(async (cx) => {
      const other = key === "summary_by" && value !== setting("summary_by");
      await setSetting(key, value);
      if (other && setting("summary_model")) { await setSetting("summary_model", ""); this.model.set_value(""); }
      cx.notify();
      if (key === "summary_by" || key === "summaries") { await this.loadModels(cx); await this.loadUsage(cx); }
    });
  }

  pickModel(name, cx) {
    const v = String(name ?? "").trim();
    if (this.model.value() !== v) this.model.set_value(v);
    if (v === setting("summary_model")) { cx.notify?.(); return; }
    cx.spawn(async (cx) => { await setSetting("summary_model", v); cx.notify(); });
  }

  // Asks the helper how many times a model ran for a summary, today and in the last 7 days. An older helper
  // without the command: no line.
  async loadUsage(cx) {
    if (setting("summaries") !== "on" || !(await sidecar.hasCommand("summary-usage"))) return;
    const day = new Date(); day.setHours(0, 0, 0, 0);
    const r = await sidecar.summaryUsage([day.getTime(), day.getTime() - 6 * 86400000]);
    const l = r.error ? [] : r.lines.filter((x) => x.type === "summary-usage");
    setShared(KEY_USAGE, l.length === 2 ? { today: l[0], week: l[1] } : null);
    cx?.notify?.();
  }

  // Asks the helper for the chosen program's models, once per program while the app runs. An older helper
  // without the command: the field alone.
  async loadModels(cx) {
    const by = setting("summary_by");
    if (setting("summaries") !== "on" || by === "own" || shared(KEY_MODELS)?.[by]?.models) return;
    if (!(await sidecar.hasCommand("summary-models"))) return;
    const r = await sidecar.summaryModels(by);
    const l = r.error ? null : r.lines.find((x) => x.type === "models");
    setShared(KEY_MODELS, { ...(shared(KEY_MODELS) ?? {}), [by]: l ? { models: l.models ?? [], source: l.source } : { error: true } });
    cx?.notify?.();
  }

  render(cx) {
    const t = theme();
    const close = div().id("settings-close").size(28).flex().items_center().justify_center().rounded(6)
      .text_color(t.muted).hover((s) => s.bg(t.hover).text_color(t.foreground))
      .on_click(() => { settingsClosed(this.overlay); if (this.overlay) closeOverlay(this.overlay); })
      .child(svg(icon("close")).size(14));
    return h_flex().id("settings-screen").size_full().items_start()
      .child(this.nav(t))
      .child(v_flex().flex_1().min_w(0).h_full()
        .child(h_flex().w_full().flex_none().px(16).pt(12).justify_end().child(close))
        .child(div().flex_1().min_h(0).w_full().child(this.content(t, cx))));
  }

  // The sections, as Herdr's Settings: the title, a row a section with its icon (the chosen one filled), and
  // where the settings are saved at the bottom.
  nav(t) {
    const go = (id, cx) => { this.section = id; cx?.notify?.(); };
    return v_flex().id("set-nav").w(196).h_full().flex_none().px(12).pt(18).pb(14).gap(2).border_r(1).border_color(t.border)
      .child(div().px(10).pb(14).text_size(18).font_semibold().text_color(t.foreground).child("Flockbook"))
      .children(SECTIONS.map((s, i) => {
        const on = this.section === s.id;
        return h_flex().id(`set-nav-${s.id}`).h(34).px(10).gap(10).items_center().rounded(8).text_size(13.5)
          .tab_index(i + 1).text_color(on ? t.foreground : t.muted)
          .when(on, (el) => el.bg(t.selected))
          .when(!on, (el) => el.hover((x) => x.bg(t.hover)))
          .focus((x) => x.border(1).border_color(t.accent))
          .on_click((_e, cx) => go(s.id, cx))
          .on_key_down((e, cx) => { if (picks(e)) { cx.stop_propagation?.(); go(s.id, cx); } })
          .child(svg(icon(s.icon)).size(15).flex_none())
          .child(div().truncate().child(s.label));
      }))
      .child(div().flex_1())
      .child(div().px(10).text_size(11).text_color(t.dim).child(`Saved in ${configPath()}. Changes apply at once.`));
  }

  content(t, _cx) {
    const bad = configBad();
    let stop = SECTIONS.length;
    // The choices: `current` is the chosen value, `pick(value, cx)` acts, `off`: nothing can be picked.
    const seg = (key, choices, pick, current = setting(key), off = bad) => h_flex().flex_none().p(2).gap(2).rounded(8).bg(t.chrome).border(1).border_color(t.border)
      .children(choices.map(([value, label, slug]) => {
        const on = current === value;
        return div().id(`set-${key}-${slug}`).h(26).px(12).flex().items_center().rounded(6).text_size(12.5)
          .tab_index(++stop).bg(on ? t.selected : t.chrome).text_color(on ? t.foreground : off ? t.dim : t.muted)
          .when(on, (el) => el.font_semibold())
          .when(!off, (el) => el.hover((x) => x.bg(on ? t.selected : t.hover)))
          .focus((x) => x.border(1).border_color(t.accent))
          .on_click((_e, cx) => { if (!off) pick(value, cx); })
          .on_key_down((e, cx) => { if (picks(e) && !off) { cx.stop_propagation?.(); pick(value, cx); } })
          .child(label);
      }));
    const heading = (text) => div().pt(18).pb(2).text_size(11.5).text_color(t.muted).child(text);
    // `line`: each "\n" starts a new line of its own.
    const about = (title, line) => v_flex().flex_1().min_w(0).gap(3)
      .child(div().text_size(13.5).text_color(t.foreground).child(title))
      .children(String(line ?? "").split("\n").map((l, i) => div().when(i > 0, (el) => el.pt(3)).text_size(12).text_color(t.muted).child(l)));
    const row = (s) => h_flex().id(`set-${s.key}`).w_full().py(14).gap(24).items_center().border_b(1).border_color(t.border)
      .child(about(s.title, s.about))
      .child(seg(s.key, s.choices, (v, cx) => this.choose(s.key, v, cx)));
    const note = (text) => div().text_size(12).text_color(t.muted).child(text);
    // A row with a wide control: the title and its line, then the control under them, then `under`.
    const stacked = (key, title, line, control, under = []) => v_flex().id(`set-${key}`).w_full().py(14).gap(10).border_b(1).border_color(t.border)
      .child(about(title, line)).child(h_flex().w_full().child(control)).children(under);
    const byRow = (s) => stacked(s.key, s.title, s.about, seg(s.key, s.choices, (v, cx) => this.choose(s.key, v, cx)), [note(s.notes[setting(s.key)])]);
    // The model: the field, and under it the program's models (those that match what is typed). A click picks one.
    const modelRow = () => {
      const by = setting("summary_by"), saved = setting("summary_model"), got = shared(KEY_MODELS)?.[by];
      const all = got?.models ?? [], hits = modelMatches(all, this.model.value(), saved), shown = hits.slice(0, MODELS_SHOWN);
      const pick = (id, name, text, on) => h_flex().id(id).w_full().h(28).px(10).gap(10).items_center().rounded(6).tab_index(++stop)
        .when(on, (el) => el.bg(t.selected)).when(!on, (el) => el.hover((x) => x.bg(t.hover)))
        .focus((x) => x.border(1).border_color(t.accent))
        .on_click((_e, cx) => this.pickModel(name, cx))
        .on_key_down((e, cx) => { if (picks(e)) { cx.stop_propagation?.(); this.pickModel(name, cx); } })
        .child(div().flex_none().text_size(12).font_family("Menlo").text_color(t.foreground).child(name || "Default"))
        .child(div().flex_1().min_w(0).truncate().text_size(11.5).text_color(t.muted).child(text));
      const field = h_flex().id("set-summary_model-field").w(360).h(30).flex_none().px(10).items_center().rounded(6)
        .bg(t.background).border(1).border_color(t.border).text_size(12).child(this.modelInput());
      const under = [];
      if (all.length) {
        under.push(v_flex().id("set-summary_model-list").w_full().gap(1)
          .child(pick("set-model-default", "", MODEL_EMPTY[by], !saved))
          .children(shown.map((m, i) => pick(`set-model-${i}`, m.id, m.note ?? "", m.id === saved))));
        if (hits.length > shown.length) under.push(note(`${hits.length - shown.length} more. Type a part of a name to see fewer.`));
        if (!hits.length) under.push(note("No model in the list has this in its name. The name is used as you typed it."));
        under.push(note(MODEL_SOURCE[got.source] ?? ""));
      } else {
        under.push(note(`Empty: ${MODEL_EMPTY[by]}.${got?.error ? " The list of models could not be read." : ""}`));
      }
      return stacked(MODEL.key, MODEL.title, MODEL.about, field, under);
    };
    // The rows under Summaries show only while it is on; each agent's own program runs its default model.
    const shows = (k) => (k !== "summary_by" && k !== MODEL.key) || (setting("summaries") === "on" && (k !== MODEL.key || setting("summary_by") !== "own"));
    const rowFor = (k) => {
      if (k === MODEL.key) return modelRow();
      const s = SETTINGS.find((x) => x.key === k);
      if (s.notes) return byRow(s);
      return k === "summaries" && setting(k) === "on" ? row({ ...s, about: [SUMMARY_SENT, usageText(shared(KEY_USAGE))].filter(Boolean).join("\n") }) : row(s);
    };
    const sec = SECTIONS.find((x) => x.id === this.section) ?? SECTIONS[0];
    const body = v_flex().w_full().gap(0);
    if (bad) body.child(div().id("set-bad").mt(12).p(10).rounded(8).bg(t.raised).border(1).border_color(t.blocked)
      .text_size(12.5).text_color(t.foreground)
      .child("config.json could not be read (it is not valid JSON), so the defaults apply. Fix the file or remove it to change these here."));
    for (const [h, keys] of sec.parts ?? []) {
      body.child(heading(h));
      for (const k of keys.filter(shows)) body.child(rowFor(k));
    }
    if (sec.id === "phone") {
      body.child(heading("PHONE PAGE"))
        .child(v_flex().id("set-phone").w_full().py(14).gap(12).border_b(1).border_color(t.border)
          .child(h_flex().w_full().gap(24).items_center()
            .child(about(PHONE.title, PHONE.about))
            .child(seg("phone", [[false, "Off", "off"], [true, "On", "on"]], (v, cx) => phoneChoose(v, cx), phoneOn(), phoneBlocked())))
          .child(phoneBody(t, this, { seg, small: smallButton(t), heading })));
    }
    if (sec.id === "updates" && updateShown()) body.child(div().pt(16).child(updateLine(t, this, true)));
    return v_flex().id("settings-root").size_full().overflow_y_scrollbar().px(32).pb(24).child(body);
  }
}
