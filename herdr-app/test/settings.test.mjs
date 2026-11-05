// Flockbook settings (<appHome>/flockbook/config.json), the Flockbook settings dialog, and the update line.
import { test, beforeEach } from "node:test";
import assert from "node:assert/strict";
import { fake, log, reset } from "./stub/herdr.mjs";
import { textOf, byId } from "./stub/ui.mjs";
import { fakeCx } from "./fake-cx.mjs";
import { agent, space } from "./fixtures.mjs";
import { setting, loadConfig, setSetting, configBad, pollUpdate, updateShown, DEFAULTS, KEY_CONFIG } from "../mods/flockbook/config.js";
import { SettingsView } from "../mods/flockbook/settings.js";
import { SideView } from "../mods/flockbook/side.js";
import { Composer } from "../mods/flockbook/composer.js";
import { ChatView } from "../mods/flockbook/chat.js";
import { command, TopView } from "../mods/flockbook/top.js";
import { toggleSidebar, peekAtEdge, KEY_SCREEN, shownScreen } from "../mods/flockbook/state.js";
import { SETTINGS } from "../mods/flockbook/settings.js";
import { tabItems, act } from "../mods/flockbook/menu.js";
import { shared } from "herdr";
import * as sidecar from "../mods/flockbook/sidecar.js";

const HOME = "/home/test/.config/herdr-app";
const CONFIG = `${HOME}/flockbook/config.json`;
const jl = (...lines) => ({ code: 0, stdout: lines.map((l) => JSON.stringify(l)).join("\n") + "\n", stderr: "" });
const header = (cmd) => ({ type: "header", protocol: 1, sidecar: "0.1.0", cmd });
const end = (ok = true) => ({ type: "end", ok });
const ONE_LINE = "curl -fsSL https://github.com/minchenlee/flockbook/releases/latest/download/install.sh | sh";
const update = (state, latest = "0.9.0") => ({ type: "update", state, current: "0.8.0", latest, command: ONE_LINE });
const t = new Proxy({}, { get: (_t, k) => `#${k}` });

// A config.json that the fake file system keeps: writes land in `file`, and a write against a stale
// snapshot is refused, as herdr does.
let file, answer;
beforeEach(() => {
  reset(); file = null; answer = update("newer");
  fake.readText = (p) => (p === CONFIG ? file : null);
  fake.writeText = (p, text, expected) => {
    if (p !== CONFIG) return null;
    if (expected !== file) throw new Error("stale");
    file = text; return null;
  };
  fake.exec = (_c, args) => {
    if (args[0] === "update-check") {
      const i = args.indexOf("--dismiss");
      if (i >= 0) answer = update("dismissed", args[i + 1]);
      return jl(header("update-check"), answer, end());
    }
    return jl(header(args[0]), end());
  };
  fake.spaces = [space("sp1")];
  fake.agents = [agent("t1", "idle")];
});

const opt = (el, key, value) => byId(el, `set-${key}-${value}`);
const press = (el) => el._h.on_click({}, fakeCx());

test("no config.json: every setting is its default, and nothing is written", async () => {
  await loadConfig();
  assert.deepEqual(DEFAULTS, { check_updates: true, status_row: "show", remote_helper: "ask" });
  for (const [k, v] of Object.entries(DEFAULTS)) assert.equal(setting(k), v, k);
  assert.equal(configBad(), false);
  assert.equal(log.writes.length, 0, "opening Flockbook never creates the file");
});

test("a partial file, or a value that is not one of the choices, is the default for that key", async () => {
  file = JSON.stringify({ status_row: "hide", remote_helper: "always", check_updates: "no" });
  await loadConfig();
  assert.equal(setting("status_row"), "hide");
  assert.equal(setting("remote_helper"), "ask");
  assert.equal(setting("check_updates"), true);
});

test("a config.json that is not JSON: defaults, a warning, and the file is left alone", async () => {
  file = "{ check_updates: false";
  const v = new SettingsView(fakeCx());
  await loadConfig();
  assert.equal(configBad(), true);
  assert.equal(setting("check_updates"), true);
  const el = v.render(fakeCx());
  assert.match(textOf(byId(el, "set-bad")), /could not be read/);
  assert.equal(await setSetting("check_updates", false), false);
  assert.equal(file, "{ check_updates: false", "a file the person broke is theirs to fix");
});

test("a change keeps the other keys and every view sees it at once", async () => {
  file = JSON.stringify({ mine: 1 });
  await loadConfig();
  const v = new SettingsView(fakeCx());
  let el = v.render(fakeCx());
  assert.equal(opt(el, "status_row", "show")._a.bg, "#selected");
  press(opt(el, "status_row", "hide"));
  await new Promise((r) => setTimeout(r, 5));
  assert.deepEqual(JSON.parse(file), { mine: 1, status_row: "hide" });
  assert.equal(setting("status_row"), "hide");
  assert.ok(log.shared.some(([k]) => k === KEY_CONFIG), "shared, so the other copies redraw");
  el = v.render(fakeCx());
  assert.equal(opt(el, "status_row", "hide")._a.bg, "#selected");
  // Someone else changed the file meanwhile: the write is not lost, it is done on the new file.
  file = JSON.stringify({ mine: 2 });
  assert.equal(await setSetting("remote_helper", "never"), true);
  assert.deepEqual(JSON.parse(file), { mine: 2, remote_helper: "never" });
});

test("the screen: three settings, each with its two choices, reachable with the keyboard", async () => {
  await loadConfig();
  const el = new SettingsView(fakeCx()).render(fakeCx());
  const text = textOf(el);
  for (const s of ["Check for updates", "Status row", "Helper on other machines"]) assert.match(text, new RegExp(s));
  const stops = [];
  for (const [k, a, b] of [["check_updates", "on", "off"], ["status_row", "show", "hide"], ["remote_helper", "ask", "never"]]) {
    for (const x of [a, b]) {
      const o = opt(el, k, x);
      assert.ok(o, `${k} ${x}`);
      assert.equal(typeof o._a.tab_index, "number", "a tab stop");
      assert.ok(o._h.on_key_down, "Enter or Space picks it");
      stops.push(o._a.tab_index);
    }
  }
  assert.deepEqual([...stops].sort((x, y) => x - y), stops, "Tab goes top to bottom");
  // Space on Off.
  const o = opt(el, "check_updates", "off");
  const cx = fakeCx();
  o._h.on_key_down({ key: "space", modifiers: {} }, cx);
  await new Promise((r) => setTimeout(r, 5));
  assert.equal(JSON.parse(file).check_updates, false);
});

// The Flockbook settings: a dialog over the window, as Herdr's own Settings (dimmed behind it; Esc or a
// click beside it closes it). Not a screen of the stack.
const settingsDialogs = () => log.overlays.filter((o) => o.props?.view === "settings");
function assertDialog(o) {
  assert.equal(o.mod, "flockbook");
  assert.equal(o.kind, "dialog", "centered, dims the window, Esc and a click beside it close it");
  assert.equal(o.frame, undefined, "Herdr's frame: rounded corners, shadow, background");
  assert.ok(!o.modal, "a click beside it closes it");
}

test("the sidebar has a Flockbook row above Settings; it and the key command open the settings dialog", () => {
  const el = new SideView(fakeCx()).render(fakeCx());
  const ids = el.kids.map((k) => k?._a?.id);
  const at = ids.indexOf("side-flockbook");
  assert.ok(at > 0 && at < ids.indexOf("side-settings"), ids.join(","));
  press(byId(el, "side-flockbook"));
  assert.equal(settingsDialogs().length, 1);
  assertDialog(settingsDialogs()[0]);
  assert.ok(!log.run.some(([n, a]) => n === "layout.select" && a?.child === "settings"), "the screen behind stays");
  // Open: the key command replaces it, so there is never a second one.
  command("settings");
  assert.equal(settingsDialogs().length, 2);
  assert.deepEqual(log.closed, ["o1"]);
});

test("the settings dialog: a close button; once closed (Esc, a click beside it, the button) it opens again", () => {
  press(byId(new SideView(fakeCx()).render(fakeCx()), "side-flockbook"));
  fake.props = { view: "settings", overlay: "o1" };
  const v = new SettingsView(fakeCx());
  const el = v.render(fakeCx());
  assert.equal(byId(el, "over-bar"), null, "no top bar in the dialog");
  press(byId(el, "settings-close"));
  assert.deepEqual(log.closed, ["o1"]);
  v.close(fakeCx());
  command("settings");
  assert.equal(settingsDialogs().length, 2);
  assert.deepEqual(log.closed, ["o1"], "nothing open to replace");
  // Esc or a click beside it: Herdr closes it and the view's close runs; the next opens fresh.
  new SettingsView(fakeCx()).close(fakeCx());
  press(byId(new SideView(fakeCx()).render(fakeCx()), "side-flockbook"));
  assert.equal(settingsDialogs().length, 3);
  assert.deepEqual(log.closed, ["o1"]);
});

test("a screen saved as the old settings screen shows the chat", () => {
  fake.shared[KEY_SCREEN] = "settings";
  assert.equal(shownScreen(), "chat");
});

test("update line: a newer release shows one line in the sidebar, Copy puts the one-line install on the clipboard", async () => {
  await loadConfig();
  await pollUpdate();
  assert.equal(updateShown()?.latest, "0.9.0");
  const el = new SideView(fakeCx()).render(fakeCx());
  const line = byId(el, "update-line");
  assert.match(textOf(line), /Flockbook 0\.9\.0 is out/);
  assert.match(textOf(byId(line, "update-copy")), /^Copy install command$/);
  assert.match(textOf(byId(line, "update-dismiss")), /^Not now$/);
  let copied = null;
  const cx = { ...fakeCx(), write_to_clipboard: (s) => { copied = s; } };
  byId(line, "update-copy")._h.on_click({}, cx);
  assert.equal(copied, ONE_LINE);
  // The settings dialog shows the command itself too.
  assert.match(textOf(new SettingsView(fakeCx()).render(fakeCx())), /install\.sh \| sh/);
});

test("update line: Not now hides it for that version (the helper remembers)", async () => {
  await loadConfig(); await pollUpdate();
  const el = new SideView(fakeCx()).render(fakeCx());
  byId(el, "update-dismiss")._h.on_click({}, fakeCx());
  await new Promise((r) => setTimeout(r, 5));
  assert.ok(log.exec.some(([, a]) => a[0] === "update-check" && a.includes("--dismiss") && a.includes("0.9.0")));
  assert.equal(updateShown(), null);
  assert.equal(byId(new SideView(fakeCx()).render(fakeCx()), "update-line"), null);
});

test("update line: nothing for current, offline or an old helper; off asks nothing at all", async () => {
  for (const a of [update("current", "0.8.0"), update("error", null), update("unknown", null)]) {
    reset(); fake.readText = () => null; fake.exec = () => jl(header("update-check"), a, end());
    await loadConfig(); await pollUpdate();
    assert.equal(updateShown(), null, a.state);
    assert.equal(byId(new SideView(fakeCx()).render(fakeCx()), "update-line"), null, a.state);
  }
  // A helper that has no update-check (usage error): nothing either.
  reset(); fake.readText = () => null; fake.exec = () => jl(header("x"), { type: "error", code: "usage", message: "commands: …" }, end(false));
  await loadConfig(); await pollUpdate();
  assert.equal(updateShown(), null);
  // Off: no call to the helper.
  reset(); fake.readText = () => JSON.stringify({ check_updates: false }); fake.exec = () => jl(header("update-check"), update("newer"), end());
  await loadConfig(); await pollUpdate();
  assert.equal(log.exec.length, 0);
  assert.equal(updateShown(), null);
});

test("update line: turning updates off hides the line at once", async () => {
  await loadConfig(); await pollUpdate();
  assert.ok(updateShown());
  await setSetting("check_updates", false);
  assert.equal(updateShown(), null);
  assert.equal(byId(new SideView(fakeCx()).render(fakeCx()), "update-line"), null);
});

test("update-check is asked at most once an hour by the mod (the helper keeps it to once a day)", async () => {
  await loadConfig();
  await pollUpdate(1_000_000);
  await pollUpdate(1_000_000 + 60_000);
  assert.equal(log.exec.filter(([, a]) => a[0] === "update-check").length, 1);
  await pollUpdate(1_000_000 + 3_600_000);
  assert.equal(log.exec.filter(([, a]) => a[0] === "update-check").length, 2);
});

test("status row: Show by default, Hide leaves out the chips above the reply box", async () => {
  const status = { type: "status", tags: [{ kind: "model", text: "Opus 5.5" }] };
  const c = new Composer(fakeCx());
  c.status.set("t1", status);
  await loadConfig();
  assert.ok(byId(c.render(fake.agents[0], t), "tags"));
  await setSetting("status_row", "hide");
  assert.equal(byId(c.render(fake.agents[0], t), "tags"), null);
  assert.ok(byId(c.render(fake.agents[0], t), "box"), "the reply box stays");
});

test("every sidecar call names this app's home, so the helper reads the same config.json", async () => {
  await sidecar.feed("t1");
  await sidecar.installHelper("box", "00");
  await sidecar.send("t1", "--home");
  await sidecar.updateCheck();
  for (const [, a] of log.exec) {
    const i = a.lastIndexOf("--home");
    assert.ok(i > 0 && a[i + 1] === HOME, a.join(" "));
  }
  const inst = log.exec[1][1];
  assert.deepEqual(inst.slice(0, 2), ["helper", "install"], "the helper's subcommand stays first");
  const send = log.exec[2][1];
  assert.equal(send.lastIndexOf("--home"), send.length - 2, "last: the helper reads the last --home, not a text that says it");
});

test("helper on other machines, Never: the chat says it is off in one plain line, no install card", async () => {
  fake.spaces = [space("sp1", { machine: "box" })];
  fake.agents = [agent("box/t7", "idle")];
  const OFF = "Flockbook does not run its helper on other machines. To change it: Flockbook settings, Helper on other machines: Ask.";
  fake.exec = (_c, args) => jl(header(args[0]), { type: "error", code: "remote-off", message: OFF }, end(false));
  const cx = fakeCx(); const v = new ChatView(cx); await cx.settle(); v.width = 900;
  const text = textOf(v.render(cx));
  assert.match(text, /Flockbook does not run its helper on other machines/);
  assert.match(text, /Flockbook settings/);
  assert.doesNotMatch(text, /Install/);
  assert.ok(!log.exec.some(([, a]) => a[0] === "helper"));
});

test("helper on other machines, Never: the tab menu has no Remove helper", async () => {
  fake.spaces = [space("sp1", { machine: "box", tabs: [{ id: "a", active: true }] })];
  fake.agents = [agent("box/t7", "idle", { tab: "a" })];
  const tab = fake.spaces[0].tabs[0];
  await loadConfig();
  assert.ok(tabItems(tab, { agent: fake.agents[0], list: [tab] }).some((i) => i?.act === "remove-helper"));
  await setSetting("remote_helper", "never");
  const items = tabItems(tab, { agent: fake.agents[0], list: [tab] });
  assert.ok(!items.some((i) => i?.act === "remove-helper"));
  // It says how to remove it instead, and takes you there.
  const hint = items.find((i) => i?.act === "settings");
  assert.match(hint.label, /choose Ask in Flockbook settings first/);
  act(hint);
  assert.equal(log.overlays.at(-1)?.props?.view, "settings", "the settings dialog");
  assertDialog(log.overlays.at(-1));
});

test("sidebar hidden: a small dot on the top bar's sidebar button, until the sidebar opens or Not now", async () => {
  const dot = () => byId(byId(new TopView(fakeCx()).render(fakeCx()), "sidebar"), "update-dot");
  await loadConfig();
  fake.shared["flockbook.sidebar"] = false;
  assert.equal(dot(), null, "nothing out yet");
  await pollUpdate();
  assert.ok(dot(), "a newer release, sidebar hidden");
  toggleSidebar(); toggleSidebar();   // opened, then hidden again: seen
  assert.equal(dot(), null);
  // A newer one again: the dot is back; the floating sidebar counts as opening it too.
  fake.shared["flockbook.update"] = update("newer", "0.10.0");
  assert.ok(dot());
  peekAtEdge({ position: { x: 1, y: 300 } });
  assert.equal(dot(), null);
  // Not now hides it as well.
  fake.shared["flockbook.update"] = update("newer", "0.11.0");
  fake.shared["flockbook.sidebar.peek"] = null;
  assert.ok(dot());
  const el = new SideView(fakeCx()).render(fakeCx());
  byId(el, "update-dismiss")._h.on_click({}, fakeCx());
  await new Promise((r) => setTimeout(r, 5));
  assert.equal(dot(), null);
});

test("the words: Status row names cost; Never says how to remove a helper", () => {
  const about = Object.fromEntries(SETTINGS.map((s) => [s.key, s.about]));
  assert.match(about.status_row, /model, context, cost, folder and branch/);
  assert.match(about.remote_helper, /To remove a helper that is already there, choose Ask first\./);
});

test("a release that comes out while the sidebar is open gets no dot when it is hidden later", async () => {
  await loadConfig();
  fake.shared["flockbook.sidebar"] = true;
  await pollUpdate();
  new SideView(fakeCx()).render(fakeCx());
  fake.shared["flockbook.sidebar"] = false;
  assert.equal(byId(byId(new TopView(fakeCx()).render(fakeCx()), "sidebar"), "update-dot"), null);
});
