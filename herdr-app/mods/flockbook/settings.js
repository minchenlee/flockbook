// The "settings" view: the Flockbook settings dialog (sidebar row "Flockbook", ⌘⇧, or the tab menu),
// over the window as Herdr's own Settings; Esc, a click beside it or its close button closes it. Three
// settings, each two choices, saved in <appHome>/flockbook/config.json (config.js), and the Phone page
// (phone.js). Tab moves through the choices; Enter or Space picks one. Also the update line, which the
// sidebar shows too.
import { div, svg } from "gpui-kit";
import { v_flex, h_flex, Input, InputState } from "gpui-base";
import { theme, props, closeOverlay } from "herdr";
import { setting, setSetting, loadConfig, configBad, configPath, updateShown, dismissUpdate } from "./config.js";
import { icon, settingsClosed } from "./state.js";
import { pollPhone, phoneTick, PHONE_TICK_MS, phoneOn, phoneBlocked, phoneChoose, phoneUser, phoneBody, setPhoneOption } from "./phone.js";

const PHONE = { title: "Phone page",
  about: "The Flockbook page on your phone, through Tailscale. On: it runs in the background, also after Herdr app quits, and starts again at login." };

export const SETTINGS = [
  { key: "check_updates", title: "Check for updates", choices: [[true, "On", "on"], [false, "Off", "off"]],
    about: "Once a day, Flockbook asks GitHub (api.github.com) for its latest release and shows a line when a newer one is out." },
  { key: "status_row", title: "Status row", choices: [["show", "Show", "show"], ["hide", "Hide", "hide"]],
    about: "The model, context, cost, folder and branch above the reply box." },
  { key: "remote_helper", title: "Helper on other machines", choices: [["ask", "Ask", "ask"], ["never", "Never", "never"]],
    about: "For a tab on another machine, the chat asks before it installs the Flockbook helper there. Never: it does not ask, and nothing runs there. To remove a helper that is already there, choose Ask first." },
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
    this.overlay = props()?.overlay ?? null;
    // The Phone page's login: saved on Enter or when the field loses the keys.
    this.login = InputState.new({ placeholder: "Empty: the login Tailscale says" });
    const save = (_e, cx) => {
      const v = this.login.value().trim();
      if (v === phoneUser()) return;
      cx.spawn(async (cx) => { await setPhoneOption("phone_user", v); cx.notify(); });
    };
    this.login.on("submit", save);
    this.login.on("blur", save);
    cx?.spawn?.(async (cx) => { await loadConfig(); this.login.set_value(phoneUser()); cx.notify(); });
    cx?.spawn?.(async (cx) => { await pollPhone(false); cx.notify(); });
    // Every 10 s; every 3 s while On waits for Serve (phone.js).
    cx?.timer?.every(PHONE_TICK_MS, (cx) => cx.spawn(async (cx) => { await phoneTick(); cx.notify(); }));
  }

  close(_cx) { settingsClosed(this.overlay); }

  loginInput() { return Input.new(this.login); }

  choose(key, value, cx) { cx.spawn(async (cx) => { await setSetting(key, value); cx.notify(); }); }

  render(cx) {
    const t = theme();
    const close = div().id("settings-close").size(28).flex().items_center().justify_center().rounded(6)
      .text_color(t.muted).hover((s) => s.bg(t.hover).text_color(t.foreground))
      .on_click(() => { settingsClosed(this.overlay); if (this.overlay) closeOverlay(this.overlay); })
      .child(svg(icon("close")).size(14));
    return v_flex().id("settings-screen").size_full()
      .child(h_flex().w_full().flex_none().px(24).pt(18).pb(4).items_center()
        .child(div().flex_1().text_size(18).font_semibold().text_color(t.foreground).child("Flockbook"))
        .child(close))
      .child(div().flex_1().min_h(0).w_full().child(this.content(t, cx)));
  }

  content(t, _cx) {
    const bad = configBad();
    let stop = 0;
    // Two choices: `current` is the chosen value, `pick(value, cx)` acts, `off`: nothing can be picked.
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
    const segment = (s) => seg(s.key, s.choices, (v, cx) => this.choose(s.key, v, cx));
    const row = (s) => h_flex().id(`set-${s.key}`).w_full().py(14).gap(24).items_center().border_b(1).border_color(t.border)
      .child(v_flex().flex_1().min_w(0).gap(3)
        .child(div().text_size(13.5).text_color(t.foreground).child(s.title))
        .child(div().text_size(12).text_color(t.muted).child(s.about)))
      .child(segment(s));
    return v_flex().id("settings-root").size_full().overflow_y_scrollbar().px(24).pb(24)
      .child(v_flex().w_full().gap(4)
        .child(div().pb(8).text_size(12).text_color(t.muted).child(`Saved in ${configPath()}. Changes apply at once.`))
        .when(bad, (el) => el.child(div().id("set-bad").p(10).rounded(8).bg(t.raised).border(1).border_color(t.blocked)
          .text_size(12.5).text_color(t.foreground)
          .child("config.json could not be read (it is not valid JSON), so the defaults apply. Fix the file or remove it to change these here.")))
        .children(SETTINGS.map(row))
        .child(v_flex().id("set-phone").w_full().py(14).gap(12).border_b(1).border_color(t.border)
          .child(h_flex().w_full().gap(24).items_center()
            .child(v_flex().flex_1().min_w(0).gap(3)
              .child(div().text_size(13.5).text_color(t.foreground).child(PHONE.title))
              .child(div().text_size(12).text_color(t.muted).child(PHONE.about)))
            .child(seg("phone", [[false, "Off", "off"], [true, "On", "on"]], (v, cx) => phoneChoose(v, cx), phoneOn(), phoneBlocked())))
          .child(phoneBody(t, this, { seg, small: smallButton(t) })))
        .when(Boolean(updateShown()), (el) => el.child(div().pt(16).child(updateLine(t, this, true)))));
  }
}
