// The Phone page section of the Flockbook screen: the phone page (the helper's `serve`) as a user service,
// through the helper's `phone status|on|off` (docs/protocol.md, phone). The helper writes and removes the
// service file and reads Tailscale; the mod never changes Tailscale settings. On or off is the service file
// itself; the login and Read-only are in config.json (phone_user, phone_read_only; phone_port is not on screen).
import { div, svg } from "gpui-kit";
import { v_flex, h_flex } from "gpui-base";
import { shared, setShared, appHome } from "herdr";
import { setSetting, configBad, KEY_CONFIG } from "./config.js";
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

// The section's body, under its title and Off/On. `seg(key, choices, pick)`: the screen's two-choice control;
// `small(id, label, act)`: its small button; `owner.loginInput()`: the login field the screen keeps.
export function phoneBody(t, owner, { seg, small }) {
  const p = phone(), l = p?.line;
  const ts = l?.tailscale ?? null;
  const on = phoneOn();
  const [said, bad] = stateText(p);
  const mono = (id, text) => div().id(id).text_size(11.5).font_family("Menlo").text_color(t.foreground).child(text);
  const copy = (id, text, label = "Copy") => small(id, owner.copied === text ? "Copied" : label, (cx) => {
    cx.write_to_clipboard?.(text); owner.copied = text; cx.notify?.();
  });
  const note = (text) => div().text_size(12).text_color(t.muted).child(text);

  const body = v_flex().w_full().gap(10).pl(2)
    .child(h_flex().id("phone-state").gap(8).items_center().text_size(12.5)
      .child(div().size(7).rounded(4).flex_none().bg(said === "Running" ? t.done : bad ? t.blocked : t.dim))
      .child(div().text_color(bad ? t.foreground : t.muted).child(said)));
  // On, Serve not set: the one step left, right under the state, where it is seen.
  if (on && ts && !l.serve?.set) {
    const command = l.serve?.command ?? "tailscale serve --bg 8787";
    body.child(div().id("phone-serve-step").text_size(12.5).text_color(t.foreground).child("One step left: run this once in a terminal."))
      .child(h_flex().gap(6).items_center().child(mono("phone-serve-command", command)).child(copy("phone-serve-copy", command)))
      .when(Boolean(l.serve?.reason), (el) => el.child(div().id("phone-serve-reason").text_size(12).text_color(t.muted).child(l.serve?.reason ?? "")))
      .child(note("Tailscale Serve puts the page on your tailnet with HTTPS (not tailscale funnel). Flockbook never changes your Tailscale settings. The address shows here once it is set."));
  }
  if (!ts) {
    body.child(note(`${l?.tailscale_error ?? "Tailscale is not available."} The phone page is reached only through Tailscale. Guide: SERVE.md`))
      .child(h_flex().gap(6).items_center().child(mono("phone-guide", GUIDE)).child(copy("phone-guide-copy", GUIDE, "Copy link")));
    return body;
  }
  // The login: empty uses the one Tailscale says.
  body.child(h_flex().w_full().gap(12).items_center()
    .child(div().w(120).flex_none().text_size(12.5).text_color(t.foreground).child("Tailscale login"))
    .child(div().flex_1().min_w(0).child(owner.loginInput()))
  ).child(note(`Only this account may open the page. Empty: ${ts.login}, the account this machine is signed in with. Now: ${l.user ?? ts.login}.`));
  body.child(h_flex().w_full().gap(12).items_center()
    .child(v_flex().flex_1().min_w(0).gap(2)
      .child(div().text_size(12.5).text_color(t.foreground).child("Read-only"))
      .child(note("The page only shows; Approve, Answer and Send are refused.")))
    .child(seg("phone_read_only", [[false, "Off", "off"], [true, "On", "on"]], (v, cx) => cx.spawn(async (cx) => { await setPhoneOption("phone_read_only", v); cx.notify(); }))));
  if (on) {
    if (l.serve?.set) {
      body.child(note("Open on your phone (signed in to the same Tailscale account):"))
        .child(h_flex().gap(6).items_center().child(mono("phone-url", l.serve.url)).child(copy("phone-url-copy", l.serve.url)))
        .when(Boolean(appRel(l.qr)), (el) => el.child(div().id("phone-qr").size(156).p(8).rounded(8).bg("#ffffff").child(svg(appRel(l.qr)).size(140).text_color("#000000"))));
    }
    if (l.service?.kind === "systemd") body.child(note("Linux: it stops when you log out, unless lingering is on (loginctl enable-linger)."));
  }
  return body;
}
