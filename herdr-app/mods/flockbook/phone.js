// The Phone page section of the Flockbook screen: the phone page (the helper's `serve`) as a user service,
// through the helper's `phone status|on|off` (docs/protocol.md, phone). The helper writes and removes the
// service file and reads Tailscale; the mod never changes Tailscale settings. On or off is the service file
// itself; the login and Read-only are in config.json (phone_user, phone_read_only; phone_port is not on screen).
import { div, svg } from "gpui-kit";
import { v_flex, h_flex } from "gpui-base";
import { shared, setShared, appHome } from "herdr";
import { setSetting, configBad, KEY_CONFIG } from "./config.js";
import { icon } from "./state.js";
import * as sidecar from "./sidecar.js";

export const KEY_PHONE = "flockbook.phone";   // { line, error, at }: the helper's last phone line, and an error
const GUIDE = "https://github.com/minchenlee/flockbook/blob/main/xplat/flockbook-modfeed/SERVE.md";
const FRESH_MS = 10_000;

const phone = () => shared(KEY_PHONE) ?? null;
// On: the service file is the settings' own (running, stopped or failed).
export const phoneOn = () => ["running", "stopped", "error"].includes(phone()?.line?.service?.state);

async function ask(what) {
  const r = await sidecar.phone(what);
  const line = r.lines.find((l) => l.type === "phone") ?? phone()?.line ?? null;
  setShared(KEY_PHONE, { line, error: r.error?.message ?? null, at: Date.now() });
  return r;
}

// The helper's picture: on the screen's start (unless just asked) and every 10 s while it shows.
export async function pollPhone(force = true) {
  const p = phone();
  if (!force && p && Date.now() - p.at < FRESH_MS) return p;
  return ask("status");
}

// On and Serve not set yet: the settings ask every 3 s, so the address shows once the command has run.
export const phoneWaitsForServe = () => phoneOn() && !phone()?.line?.serve?.set && Boolean(phone()?.line?.tailscale);
export const PHONE_TICK_MS = 3_000;
export const phoneTick = () => pollPhone(phoneWaitsForServe());

export const setPhone = (on) => ask(on ? "on" : "off");

// A login or Read-only change: saved, and while on the service is written again with it.
export async function setPhoneOption(key, value) {
  if (!(await setSetting(key, value))) return false;
  await ask(phoneOn() ? "on" : "status");
  return true;
}

// svg() reads a path under the app root, which is the app home (the icons are "mods/flockbook/…"), not an
// absolute one. The helper writes the QR in <app home>/flockbook (the mod passes --home): the path from there.
// Null when it is not under the app home.
export function appRel(path) {
  const root = appHome().replace(/\/+$/, "") + "/";
  return typeof path === "string" && path.startsWith(root) && path.length > root.length ? path.slice(root.length) : null;
}

// What the status line says, and whether it is a problem.
function stateText(p) {
  const l = p?.line, s = l?.service;
  if (p?.error) return [p.error, true];
  if (!l) return ["Asking the helper…", false];
  if (!s) return ["The helper did not say.", true];
  if (s.state === "running") return ["Running", false];
  if (s.state === "stopped") return [s.message ? `Not running. ${s.message}` : "Not running", true];
  if (s.state === "error") return [s.message ?? "It stopped with an error.", true];
  if (s.state === "by-hand") return [s.message ?? "Set up by hand (see SERVE.md); Flockbook settings leave it alone.", false];
  return ["Off", false];
}

// Off or On can be chosen: Tailscale is there, the service is not one set up by hand, config.json reads.
export function phoneBlocked() {
  const l = phone()?.line;
  return !l?.tailscale || l?.service?.state === "by-hand" || configBad();
}

export function phoneChoose(want, cx) {
  if (phoneBlocked() || want === phoneOn()) return;
  cx.spawn(async (cx) => { await setPhone(want); cx.notify(); });
}

// The login in config.json; empty: the one Tailscale says.
export function phoneUser() {
  const v = shared(KEY_CONFIG)?.file?.phone_user;
  return typeof v === "string" ? v : "";
}

const GET_TAILSCALE = "https://tailscale.com/download";

// The section's body, under its title and Off/On: the state line when there is something to say, then SET UP
// (four steps, each done, the one to do now, or later and dim) and OPTIONS (the login and Read-only).
// `seg(key, choices, pick)`: the screen's two-choice control; `small(id, label, act)`: its small button;
// `heading(text)`: its UPPERCASE heading; `owner.loginInput()`: the login field the screen keeps.
export function phoneBody(t, owner, { seg, small, heading }) {
  const p = phone(), l = p?.line;
  const ts = l?.tailscale ?? null;
  const on = phoneOn();
  const running = l?.service?.state === "running";
  const user = l?.user ?? ts?.login ?? "";
  const [said, bad] = stateText(p);
  const mono = (id, text) => div().id(id).text_size(11.5).font_family("Menlo").text_color(t.foreground).child(text);
  const copy = (id, text, label = "Copy") => small(id, owner.copied === text ? "Copied" : label, (cx) => {
    cx.write_to_clipboard?.(text); owner.copied = text; cx.notify?.();
  });
  const note = (text) => div().text_size(12).text_color(t.muted).child(text);
  const line = (text) => div().text_size(12.5).text_color(t.foreground).child(text);
  const withCopy = (id, text, label) => h_flex().gap(6).items_center().flex_wrap().child(mono(id, text)).child(copy(`${id}-copy`, text, label));

  const body = v_flex().w_full().gap(10).pl(2);
  // The state, with a red dot for a problem. Off says nothing here: the switch and step 2 say it.
  if (said !== "Off" || bad) {
    body.child(h_flex().id("phone-state").gap(8).items_center().text_size(12.5)
      .child(div().size(7).rounded(4).flex_none().bg(said === "Running" ? t.done : bad ? t.blocked : t.dim))
      .child(div().text_color(bad ? t.foreground : t.muted).child(said)));
  }

  // One step: a round mark (a check when done, its number otherwise), the title, and what it says.
  const step = (n, title, state, ...lines) => {
    const mark = div().id(`phone-step-${n}-${state}`).size(20).flex_none().flex().items_center().justify_center().rounded(10)
      .border(1).text_size(11).font_semibold()
      .border_color(state === "done" ? t.done : state === "now" ? t.accent : t.border)
      .text_color(state === "done" ? t.done : state === "now" ? t.accent : t.dim)
      .child(state === "done" ? svg(icon("check")).size(11) : String(n));
    return h_flex().id(`phone-step-${n}`).w_full().gap(12).items_start()
      .child(mark)
      .child(v_flex().flex_1().min_w(0).gap(6).pt(1)
        .child(div().text_size(13).text_color(state === "later" ? t.dim : t.foreground).child(title))
        .when(state !== "later", (el) => el.children(lines.filter(Boolean))));
  };
  const s1 = ts ? "done" : "now";
  const s2 = !ts ? "later" : running ? "done" : "now";
  const set = Boolean(l?.serve?.set);
  const s3 = s2 !== "done" ? "later" : set ? "done" : "now";
  const s4 = s3 === "done" ? "now" : "later";

  const steps = v_flex().w_full().gap(14);
  steps.child(step(1, "Tailscale on this computer", s1,
    ts ? note(`Signed in as ${ts.login}.`) : null,
    ts ? null : line(l?.tailscale_error ?? "Tailscale is not available."),
    ts ? null : note("The phone page is reached only through Tailscale. Install it, sign in, and this step is done by itself."),
    ts ? null : h_flex().gap(6).child(small("phone-get-tailscale", "Get Tailscale", (cx) => cx.open_url?.(GET_TAILSCALE))),
    ts ? null : note("The setup guide:"),
    ts ? null : withCopy("phone-guide", GUIDE, "Copy link")));
  const by = l?.service?.state === "by-hand";
  steps.child(step(2, "Phone page on", s2,
    running ? note("It runs in the background, also after Herdr quits, and starts again at login.") : null,
    by ? note(said) : null,
    !running && !by && !on ? note("Turn it on with the switch above. It runs in the background, also after Herdr quits.") : null,
    !running && on ? note("Not running. To try again, turn it Off, then On.") : null,
    on && l?.service?.kind === "systemd" ? note("Linux: it stops when you log out, unless lingering is on (loginctl enable-linger).") : null));
  const command = l?.serve?.command ?? "tailscale serve --bg 8787";
  steps.child(step(3, "Tailscale Serve set", s3,
    s3 === "now" ? line("Run this once in a terminal on this computer:") : null,
    s3 === "now" ? h_flex().id("phone-serve-step").gap(6).items_center().child(mono("phone-serve-command", command)).child(copy("phone-serve-copy", command)) : null,
    s3 === "now" && l?.serve?.reason ? div().id("phone-serve-reason").text_size(12).text_color(t.muted).child(l.serve.reason) : null,
    s3 === "now" ? note("Tailscale Serve puts the phone page on your tailnet with HTTPS (not tailscale funnel). Flockbook never changes your Tailscale settings. This step is done by itself once Serve is set.") : null));
  steps.child(step(4, "Open it on your phone", s4,
    line(`On your phone: install Tailscale and sign in as ${user}. Then scan this code or open the address.`),
    set ? h_flex().w_full().gap(16).items_center()
      .when(Boolean(appRel(l.qr)), (el) => el.child(div().id("phone-qr").size(156).p(8).rounded(8).bg("#ffffff").child(svg(appRel(l.qr)).size(140).text_color("#000000"))))
      .child(withCopy("phone-url", l.serve.url)) : null,
    note(`Can't open it? Turn Tailscale on in the phone, and check that it is signed in as ${user}.`)));
  body.child(heading("SET UP")).child(steps);
  if (!ts) return body;

  // OPTIONS: rows as the settings' own, the title and its line on the left, the control on the right. The
  // login field sits in a frame as Herdr's own (`background`, a `border` line), its text as the line's.
  body.child(heading("OPTIONS"));
  body.child(h_flex().id("phone-login-row").w_full().gap(12).items_center()
    .child(v_flex().flex_1().min_w(0).gap(2)
      .child(div().text_size(12.5).text_color(t.foreground).child("Tailscale login"))
      .child(note(`Only this account may open the page. Now: ${user}.`)))
    .child(h_flex().id("phone-login").w(260).h(30).flex_none().px(10).items_center().rounded(6)
      .bg(t.background).border(1).border_color(t.border).text_size(12).child(owner.loginInput())));
  body.child(h_flex().w_full().gap(12).items_center()
    .child(v_flex().flex_1().min_w(0).gap(2)
      .child(div().text_size(12.5).text_color(t.foreground).child("Read-only"))
      .child(note("The page only shows; Approve, Answer and Send are refused.")))
    .child(seg("phone_read_only", [[false, "Off", "off"], [true, "On", "on"]], (v, cx) => cx.spawn(async (cx) => { await setPhoneOption("phone_read_only", v); cx.notify(); }))));
  return body;
}
