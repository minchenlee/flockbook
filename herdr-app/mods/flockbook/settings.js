// The "settings" view: the Flockbook screen (sidebar row "Flockbook", or ⌘⇧,). Three settings, each two
// choices, saved in <appHome>/flockbook/config.json (config.js). Tab moves through the choices; Enter or
// Space picks one. Also the update line, which the sidebar shows too.
import { div } from "gpui-kit";
import { v_flex, h_flex } from "gpui-base";
import { theme } from "herdr";
import { setting, setSetting, loadConfig, configBad, configPath, updateShown, dismissUpdate } from "./config.js";
import { watchEdge, noteWindowHeight } from "./state.js";
import { overBar, startTop, BAR_H } from "./top.js";

export const SETTINGS = [
  { key: "check_updates", title: "Check for updates", choices: [[true, "On", "on"], [false, "Off", "off"]],
    about: "Once a day, Flockbook asks GitHub (api.github.com) for its latest release and shows a line when a newer one is out." },
  { key: "status_row", title: "Status row", choices: [["show", "Show", "show"], ["hide", "Hide", "hide"]],
    about: "The model, context, folder and branch above the reply box." },
  { key: "remote_helper", title: "Helper on other machines", choices: [["ask", "Ask", "ask"], ["never", "Never", "never"]],
    about: "For a tab on another machine, the chat asks before it installs the Flockbook helper there. Never: it does not ask, and nothing runs there." },
];

const picks = (e) => e?.key === "enter" || e?.key === "space" || e?.key === " ";

// "Flockbook X is out" with Copy (the one-line install) and Not now. Null when there is nothing to say.
// `full`: the command is written out too (the Flockbook screen; the sidebar is narrow).
export function updateLine(t, owner, full = false) {
  const u = updateShown();
  if (!u) return null;
  const small = (id, label, act) => div().id(id).h(24).px(9).flex().items_center().rounded(6).text_size(12)
    .text_color(t.foreground).bg(t.chrome).border(1).border_color(t.border).hover((s) => s.bg(t.hover))
    .on_mouse_down("left", (_e, cx) => cx.stop_propagation()).on_click((_e, cx) => act(cx)).child(label);
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
    this.top = {}; startTop(cx, this.top);
    cx?.spawn?.(async (cx) => { await loadConfig(); cx.notify(); });
  }

  choose(key, value, cx) { cx.spawn(async (cx) => { await setSetting(key, value); cx.notify(); }); }

  render(cx) {
    const t = theme();
    const stop = (_e, cx) => cx.stop_propagation();
    return v_flex().id("settings-screen").relative().size_full().bg(t.background).on_size_change((size) => noteWindowHeight(size))
      .child(div().size_full().pt(BAR_H).on_mouse_down("left", stop).child(this.content(t, cx)))
      .child(overBar(t));
  }

  content(t, _cx) {
    const bad = configBad();
    let stop = 0;
    const segment = (s) => h_flex().flex_none().p(2).gap(2).rounded(8).bg(t.chrome).border(1).border_color(t.border)
      .children(s.choices.map(([value, label, slug]) => {
        const on = setting(s.key) === value;
        return div().id(`set-${s.key}-${slug}`).h(26).px(12).flex().items_center().rounded(6).text_size(12.5)
          .tab_index(++stop).bg(on ? t.selected : t.chrome).text_color(on ? t.foreground : t.muted)
          .when(on, (el) => el.font_semibold())
          .when(!bad, (el) => el.hover((x) => x.bg(on ? t.selected : t.hover)))
          .focus((x) => x.border(1).border_color(t.accent))
          .on_click((_e, cx) => { if (!bad) this.choose(s.key, value, cx); })
          .on_key_down((e, cx) => { if (picks(e) && !bad) { cx.stop_propagation?.(); this.choose(s.key, value, cx); } })
          .child(label);
      }));
    const row = (s) => h_flex().id(`set-${s.key}`).w_full().py(14).gap(24).items_center().border_b(1).border_color(t.border)
      .child(v_flex().flex_1().min_w(0).gap(3)
        .child(div().text_size(13.5).text_color(t.foreground).child(s.title))
        .child(div().text_size(12).text_color(t.muted).child(s.about)))
      .child(segment(s));
    return watchEdge(v_flex().id("settings-root").size_full().items_center().overflow_y_scrollbar().px(32).py(24))
      .child(v_flex().w_full().max_w(640).gap(4)
        .child(div().text_size(18).font_semibold().text_color(t.foreground).child("Flockbook"))
        .child(div().pb(8).text_size(12).text_color(t.muted).child(`Saved in ${configPath()}. Changes apply at once.`))
        .when(bad, (el) => el.child(div().id("set-bad").p(10).rounded(8).bg(t.raised).border(1).border_color(t.blocked)
          .text_size(12.5).text_color(t.foreground)
          .child("config.json could not be read (it is not valid JSON), so the defaults apply. Fix the file or remove it to change these here.")))
        .children(SETTINGS.map(row))
        .when(Boolean(updateShown()), (el) => el.child(div().pt(16).child(updateLine(t, this, true)))));
  }
}
