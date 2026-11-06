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
let file, ts, state, url, fail, reason, qrAt;
const line = () => ({
  type: "phone",
  tailscale: ts, tailscale_error: ts ? null : "Tailscale is not installed.",
  user: JSON.parse(file ?? "{}").phone_user || ts?.login || null,
  port: 8787, read_only: JSON.parse(file ?? "{}").phone_read_only === true,
  serve: { set: Boolean(url), url, command: reason ? "tailscale serve --bg --https=8443 8787" : "tailscale serve --bg 8787", reason: reason ?? null },
  qr: url ? (qrAt ?? `${HOME}/flockbook/phone-qr-1a2b3c4d.svg`) : null,
  service: { kind: "systemd", state, file: "/home/test/.config/systemd/user/flockbook-serve.service",
    message: state === "error" ? "It failed. Its log: journalctl --user -u flockbook-serve" : state === "by-hand" ? "Set up by hand (see SERVE.md)." : null },
});
const phoneCalls = () => log.exec.filter(([, a]) => a[0] === "phone").map(([, a]) => a.slice(0, a.indexOf("--home")).join(" "));

beforeEach(() => {
  reset(); file = null; ts = { login: "me@example.com", dns: "box.tail1.ts.net" }; state = "off"; url = null; fail = null; reason = null; qrAt = null;
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
  assert.ok(at(/Running/) >= 0 && at(/Run this once in a terminal on this computer/) > at(/Running/), text);
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
  // svg() reads a path under the app root (the app home), not an absolute one.
  assert.equal(byId(s, "phone-qr").kids[0]._a.path, "flockbook/phone-qr-1a2b3c4d.svg");
});

test("a QR file outside the app home is not drawn (svg() could not find it)", async () => {
  state = "running"; url = "https://box.tail1.ts.net/"; qrAt = "/elsewhere/flockbook/phone-qr-1a2b3c4d.svg";
  const { el } = await screen();
  const s = byId(el(), "set-phone");
  assert.equal(textOf(byId(s, "phone-url")), "https://box.tail1.ts.net/", "the address still shows");
  assert.equal(byId(s, "phone-qr"), null);
});

test("Off stops it; an error says what failed and where to look", async () => {
  state = "running";
  const { el } = await screen();
  click(el(), "set-phone-off");
  await settle(); await settle();
  assert.deepEqual(phoneCalls(), ["phone status", "phone off"]);
  assert.equal(byId(el(), "phone-state"), null, "Off: no state line, the switch and step 2 say it");
  assert.ok(byId(el(), "phone-step-2-now"));
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

test("the login field looks like a field: a frame and background, the row line's text size, a plain placeholder", async () => {
  const { v, el } = await screen();
  const s = byId(el(), "set-phone");
  const f = byId(s, "phone-login");
  assert.ok(f, "a frame around the field");
  assert.equal(f._a.border, 1);
  assert.equal(f._a.border_color, "#border");
  assert.equal(f._a.bg, "#background");
  assert.equal(typeof f._a.rounded, "number");
  assert.equal(f._a.text_size, 12, "as the line under a row's title");
  assert.ok(f.kids.some((k) => k.tag === "input"), "the input inside");
  assert.equal(v.login.placeholder, "Default: this machine's Tailscale login");
  const words = textOf(s);
  assert.doesNotMatch(words, /Empty/);
  assert.match(words, /Only this account may open the page\. Now: me@example\.com\./);
  // As the Read-only row: the title and its line on the left, the field on the right.
  const row = byId(s, "phone-login-row");
  assert.match(textOf(row.kids[0]), /^Tailscale login Only this account/);
  assert.equal(row.kids[1], f);
});

// The setup guide: four steps, each done, the one to do now, or later (dim), under SET UP; the options after.
const stepState = (s, n) => ["done", "now", "later"].find((k) => byId(s, `phone-step-${n}-${k}`));
const steps = (s) => [1, 2, 3, 4].map((n) => stepState(s, n));
const order = (s, ...res) => { const t = textOf(s); return res.map((r) => t.search(r)); };

test("guide, Off: Tailscale done with the login; turning the page on is the step now; Serve and the phone later", async () => {
  const { el } = await screen();
  const s = byId(el(), "set-phone");
  assert.deepEqual(steps(s), ["done", "now", "later", "later"]);
  assert.match(textOf(byId(s, "phone-step-1")), /Tailscale on this computer.*Signed in as me@example\.com\./);
  assert.match(textOf(byId(s, "phone-step-2")), /Phone page on.*Turn it on with the switch above\./);
  assert.equal(byId(s, "phone-serve-command"), null, "a later step shows no command");
  const [setUp, options, login] = order(s, /SET UP/, /OPTIONS/, /Tailscale login/);
  assert.ok(setUp >= 0 && setUp < options && options < login, "SET UP, then OPTIONS with the login");
  assert.match(textOf(s), /Read your agents on your phone, through Tailscale\./);
});

test("guide, no Tailscale: step 1 says why, links Get Tailscale and the full guide address; no options", async () => {
  ts = null;
  const { el } = await screen();
  const s = byId(el(), "set-phone");
  assert.deepEqual(steps(s), ["now", "later", "later", "later"]);
  const one = byId(s, "phone-step-1");
  assert.match(textOf(one), /Tailscale is not installed\./);
  let opened = null;
  byId(one, "phone-get-tailscale")._h.on_click({}, { ...fakeCx(), open_url: (u) => { opened = u; } });
  assert.equal(opened, "https://tailscale.com/download");
  assert.equal(textOf(byId(one, "phone-guide")), "https://github.com/minchenlee/flockbook/blob/main/xplat/flockbook-modfeed/SERVE.md");
  assert.ok(byId(one, "phone-guide-copy"));
  assert.doesNotMatch(textOf(s), /Guide: SERVE\.md/, "not a bare file name");
  assert.equal(byId(s, "phone-login-row"), null);
  assert.doesNotMatch(textOf(s), /OPTIONS/);
});

test("guide, On and Serve missing: steps 1 and 2 done; step 3 is the command to run, with Copy", async () => {
  state = "running";
  const { el } = await screen();
  const s = byId(el(), "set-phone");
  assert.deepEqual(steps(s), ["done", "done", "now", "later"]);
  const three = byId(s, "phone-step-3");
  assert.match(textOf(three), /Tailscale Serve set.*Run this once in a terminal on this computer:/);
  assert.ok(byId(three, "phone-serve-command") && byId(three, "phone-serve-copy"));
  assert.match(textOf(three), /This step is done by itself once Serve is set\./);
  assert.match(textOf(byId(s, "phone-step-2")), /also after Herdr quits/);
});

test("guide, set up: steps 1 to 3 done; step 4 is the phone: sign in as the login, the QR and the address, help", async () => {
  state = "running"; url = "https://box.tail1.ts.net/";
  const { el } = await screen();
  const s = byId(el(), "set-phone");
  assert.deepEqual(steps(s), ["done", "done", "done", "now"]);
  const four = byId(s, "phone-step-4");
  assert.match(textOf(four), /On your phone: install Tailscale and sign in as me@example\.com\. Then scan this code or open the address\./);
  assert.ok(byId(four, "phone-qr") && byId(four, "phone-url") && byId(four, "phone-url-copy"));
  assert.match(textOf(four), /Can't open it\? Turn Tailscale on in the phone, and check that it is signed in as me@example\.com\./);
  const [qr, options] = order(s, /On your phone/, /OPTIONS/);
  assert.ok(qr < options, "the goal comes before the options");
});

test("guide, stopped with an error: the red line is at the top; step 2 is the step now and says how to try again", async () => {
  state = "error";
  const { el } = await screen();
  const s = byId(el(), "set-phone");
  const st = byId(s, "phone-state");
  assert.match(textOf(st), /journalctl --user -u flockbook-serve/);
  assert.equal(st.kids[0]._a.bg, "#blocked", "a red dot");
  const [top, setUp] = order(s, /journalctl/, /SET UP/);
  assert.ok(top < setUp, "above the steps");
  assert.deepEqual(steps(s), ["done", "now", "later", "later"]);
  assert.match(textOf(byId(s, "phone-step-2")), /Not running\. To try again, turn it Off, then On\./);
});
