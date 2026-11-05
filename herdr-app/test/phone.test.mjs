// The Phone page section of the Flockbook settings: the helper's `phone status|on|off` (docs/protocol.md, phone).
import { test, beforeEach } from "node:test";
import assert from "node:assert/strict";
import { fake, log, reset } from "./stub/herdr.mjs";
import { textOf, byId } from "./stub/ui.mjs";
import { fakeCx } from "./fake-cx.mjs";
import { agent, space } from "./fixtures.mjs";
import { loadConfig } from "../mods/flockbook/config.js";
import { SettingsView } from "../mods/flockbook/settings.js";
import { pollPhone, phoneOn } from "../mods/flockbook/phone.js";

const HOME = "/home/test/.config/herdr-app";
const CONFIG = `${HOME}/flockbook/config.json`;
const jl = (...lines) => ({ code: 0, stdout: lines.map((l) => JSON.stringify(l)).join("\n") + "\n", stderr: "" });
const header = (cmd) => ({ type: "header", protocol: 1, sidecar: "0.1.0", cmd });
const end = (ok = true) => ({ type: "end", ok });

// The fake helper: Tailscale signed in unless `ts` is null; the service as `state`; Serve set when `url`.
let file, ts, state, url, fail, reason;
const line = () => ({
  type: "phone",
  tailscale: ts, tailscale_error: ts ? null : "Tailscale is not installed.",
  user: JSON.parse(file ?? "{}").phone_user || ts?.login || null,
  port: 8787, read_only: JSON.parse(file ?? "{}").phone_read_only === true,
  serve: { set: Boolean(url), url, command: reason ? "tailscale serve --bg --https=8443 8787" : "tailscale serve --bg 8787", reason: reason ?? null },
  qr: url ? `${HOME}/flockbook/phone-qr.svg` : null,
  service: { kind: "systemd", state, file: "/home/test/.config/systemd/user/flockbook-serve.service",
    message: state === "error" ? "It failed. Its log: journalctl --user -u flockbook-serve" : state === "by-hand" ? "Set up by hand (see SERVE.md)." : null },
});
const phoneCalls = () => log.exec.filter(([, a]) => a[0] === "phone").map(([, a]) => a.slice(0, a.indexOf("--home")).join(" "));

beforeEach(() => {
  reset(); file = null; ts = { login: "me@example.com", dns: "box.tail1.ts.net" }; state = "off"; url = null; fail = null; reason = null;
  fake.readText = (p) => (p === CONFIG ? file : null);
  fake.writeText = (p, text, expected) => { if (p !== CONFIG) return null; if (expected !== file) throw new Error("stale"); file = text; return null; };
  fake.exec = (_c, args) => {
    if (args[0] !== "phone") return jl(header(args[0]), end());
    assert.equal(args.at(-2), "--home", "--home last");
    assert.equal(args.at(-1), HOME);
    if (args[1] === "on") {
      if (!ts) return jl(header("phone"), line(), { type: "error", code: "no-tailscale", message: "Tailscale is not installed." }, end(false));
      if (fail) return jl(header("phone"), line(), { type: "error", code: "service", message: fail }, end(false));
      state = "running";
    }
    if (args[1] === "off") state = "off";
    return jl(header("phone"), line(), end());
  };
  fake.spaces = [space("sp1")];
  fake.agents = [agent("t1", "idle")];
});

async function screen() {
  await loadConfig();
  await pollPhone();
  const v = new SettingsView(fakeCx()); v.section = "phone";
  return { v, el: () => v.render(fakeCx()) };
}
const click = (el, id) => byId(el, id)._h.on_click({}, fakeCx());
const settle = () => new Promise((r) => setTimeout(r, 10));

test("off: the section names the Tailscale login and how the page is reached, and On is there", async () => {
  const { el } = await screen();
  const s = byId(el(), "set-phone");
  assert.ok(s, "a Phone page section");
  assert.match(textOf(s), /Phone page/);
  assert.match(textOf(s), /Tailscale/);
  assert.match(textOf(s), /me@example\.com/, "the login found");
  assert.ok(byId(s, "set-phone-off")._a.bg === "#selected", "Off is chosen");
  assert.equal(byId(s, "phone-serve-command"), null, "no Serve step while off");
  assert.deepEqual(phoneCalls(), ["phone status"]);
});

test("no Tailscale: On does nothing, and the section says why and where the guide is", async () => {
  ts = null;
  const { el } = await screen();
  const s = byId(el(), "set-phone");
  assert.match(textOf(s), /Tailscale is not installed/);
  assert.match(textOf(s), /SERVE\.md/);
  click(s, "set-phone-on");
  await settle();
  assert.deepEqual(phoneCalls(), ["phone status"], "no phone on");
});

test("On runs the page as a service; the status says Running and shows the one Serve command to copy", async () => {
  const { v, el } = await screen();
  click(el(), "set-phone-on");
  await settle(); await settle();
  assert.deepEqual(phoneCalls(), ["phone status", "phone on"]);
  assert.ok(phoneOn());
  const s = byId(el(), "set-phone");
  assert.match(textOf(byId(s, "phone-state")), /Running/);
  assert.equal(textOf(byId(s, "phone-serve-command")), "tailscale serve --bg 8787");
  const cx = { ...fakeCx(), write_to_clipboard: (x) => { cx.copied = x; } };
  byId(s, "phone-serve-copy")._h.on_click({}, cx);
  assert.equal(cx.copied, "tailscale serve --bg 8787");
  assert.match(textOf(s), /never changes your Tailscale settings/i);
  assert.match(textOf(s), /loginctl enable-linger/, "systemd: it stops at logout without lingering");
  void v;
});

test("On, Serve not set: the step is right under Running, above the login and Read-only", async () => {
  state = "running";
  const { el } = await screen();
  const s = byId(el(), "set-phone");
  const text = textOf(s);
  const at = (re) => text.search(re);
  assert.ok(at(/Running/) >= 0 && at(/One step left: run this once in a terminal/) > at(/Running/), text);
  assert.ok(at(/tailscale serve --bg 8787/) < at(/Tailscale login/), "the command before the login field");
  assert.ok(at(/tailscale serve --bg 8787/) < at(/Read-only/), "and before Read-only");
  assert.ok(byId(s, "phone-serve-copy"));
  assert.equal(byId(s, "phone-serve-reason"), null, "443 free: no reason line");
});

test("443 used by another service: the 8443 command and one line that says why", async () => {
  state = "running"; reason = "443 is used by another service on this machine, so the page goes on 8443; the plain command would replace it.";
  const { el } = await screen();
  const s = byId(el(), "set-phone");
  assert.equal(textOf(byId(s, "phone-serve-command")), "tailscale serve --bg --https=8443 8787");
  assert.match(textOf(byId(s, "phone-serve-reason")), /443 is used by another service/);
  const cx = { ...fakeCx(), write_to_clipboard: (x) => { cx.copied = x; } };
  byId(s, "phone-serve-copy")._h.on_click({}, cx);
  assert.equal(cx.copied, "tailscale serve --bg --https=8443 8787");
});

test("while On waits for Serve, the status is asked every 3 s; the address and QR show without reopening", async () => {
  state = "running";
  await loadConfig();
  await pollPhone();
  const cx = fakeCx();
  const v = new SettingsView(cx); v.section = "phone";
  await cx.settle();
  const tick = cx.timers.find((x) => !x.once && x.ms <= 3000);
  assert.ok(tick, "a timer of 3 s or less");
  const fire = async () => { const c = fakeCx(); tick.f(c); await c.settle(); };
  const before = phoneCalls().length;
  await fire();
  assert.equal(phoneCalls().length, before + 1, "asked again at once while waiting");
  url = "https://box.tail1.ts.net/";
  await fire();
  const s = byId(v.render(fakeCx()), "set-phone");
  assert.equal(textOf(byId(s, "phone-url")), "https://box.tail1.ts.net/");
  assert.ok(byId(s, "phone-qr"));
  assert.equal(byId(s, "phone-serve-command"), null);
  // Set: back to every 10 s (the last answer is fresh).
  const n = phoneCalls().length;
  await fire();
  assert.equal(phoneCalls().length, n, "not asked again within 10 s once set");
});

test("once Serve is set: the address to open, Copy, and a QR code", async () => {
  state = "running"; url = "https://box.tail1.ts.net/";
  const { el } = await screen();
  const s = byId(el(), "set-phone");
  assert.equal(byId(s, "phone-serve-command"), null);
  assert.equal(textOf(byId(s, "phone-url")), "https://box.tail1.ts.net/");
  const cx = { ...fakeCx(), write_to_clipboard: (x) => { cx.copied = x; } };
  byId(s, "phone-url-copy")._h.on_click({}, cx);
  assert.equal(cx.copied, "https://box.tail1.ts.net/");
  assert.equal(byId(s, "phone-qr").kids[0]._a.path, `${HOME}/flockbook/phone-qr.svg`);
});

test("Off stops it; an error says what failed and where to look", async () => {
  state = "running";
  const { el } = await screen();
  click(el(), "set-phone-off");
  await settle(); await settle();
  assert.deepEqual(phoneCalls(), ["phone status", "phone off"]);
  assert.match(textOf(byId(el(), "phone-state")), /Not running|Off/);
  fail = "systemctl --user restart failed (1): no bus";
  click(el(), "set-phone-on");
  await settle(); await settle();
  assert.match(textOf(byId(el(), "phone-state")), /no bus/);
  state = "error";
  await pollPhone();
  assert.match(textOf(byId(el(), "phone-state")), /journalctl --user -u flockbook-serve/);
});

test("set up by hand: the choices do nothing and the section says it leaves it alone", async () => {
  state = "by-hand";
  const { el } = await screen();
  assert.match(textOf(byId(el(), "set-phone")), /by hand/);
  click(el(), "set-phone-off");
  click(el(), "set-phone-on");
  await settle();
  assert.deepEqual(phoneCalls(), ["phone status"]);
});

test("the login and Read-only go into config.json; while on, the service is written again", async () => {
  state = "running";
  const { v, el } = await screen();
  assert.equal(v.login.value(), "", "empty: the login found is used");
  v.login.set_value("boss@example.com");
  v.login._h.submit({}, fakeCx());
  await settle(); await settle();
  assert.equal(JSON.parse(file).phone_user, "boss@example.com");
  click(el(), "set-phone_read_only-on");
  await settle(); await settle();
  assert.equal(JSON.parse(file).phone_read_only, true);
  assert.deepEqual(phoneCalls(), ["phone status", "phone on", "phone on"]);
  assert.match(textOf(byId(el(), "set-phone")), /boss@example\.com/);
});

test("off: a login change is saved, and nothing is started", async () => {
  const { v } = await screen();
  v.login.set_value("boss@example.com");
  v.login._h.blur({}, fakeCx());
  await settle(); await settle();
  assert.equal(JSON.parse(file).phone_user, "boss@example.com");
  assert.deepEqual(phoneCalls(), ["phone status", "phone status"]);
});

test("keyboard: Enter on On turns it on", async () => {
  const { el } = await screen();
  const on = byId(el(), "set-phone-on");
  assert.ok(on._a.tab_index > 0);
  on._h.on_key_down({ key: "enter" }, { ...fakeCx(), stop_propagation() {} });
  await settle(); await settle();
  assert.deepEqual(phoneCalls(), ["phone status", "phone on"]);
});
