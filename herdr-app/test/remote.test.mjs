// Terminals on another machine (`box/t7`): the helper there, asked for before it is put there.
import { test, beforeEach } from "node:test";
import assert from "node:assert/strict";
import { fake, log, reset } from "./stub/herdr.mjs";
import { textOf, byId } from "./stub/ui.mjs";
import { fakeCx } from "./fake-cx.mjs";
import { agent, space } from "./fixtures.mjs";
import { ChatView } from "../mods/flockbook/chat.js";
import { tabItems, act } from "../mods/flockbook/menu.js";
import { machineOf, KEY_REMOVE } from "../mods/flockbook/state.js";

const jl = (...lines) => ({ code: 0, stdout: lines.map((l) => JSON.stringify(l)).join("\n") + "\n", stderr: "" });
const header = (command) => ({ type: "header", protocol: 1, sidecar: "0.1.0", command });
const end = (ok = true) => ({ type: "end", ok });
const SHA = "7ac5" + "0".repeat(56) + "beef";
const install = (extra = {}) => ({ machine: "box", target: "x86_64-unknown-linux-musl", path: "~/.local/share/flockbook/helper/0.1.0/flockbook-modfeed",
  bytes: 4371392, sha256: SHA, source: "release", ...extra });
const missing = (inst = install()) => ({ type: "error", code: "remote_helper_missing", message: "box has no Flockbook helper yet, so the chat cannot read its agents.", install: inst });

let there, calls;
beforeEach(() => {
  reset(); calls = []; there = false;
  fake.spaces = [space("sp1", { machine: "box" })];
  fake.agents = [agent("box/t7", "idle")];
  fake.exec = (_c, args) => {
    calls.push(args);
    if (args[0] === "helper" && args[1] === "install") { there = true; return jl(header("helper"), { type: "installed", machine: "box", path: install().path, bytes: 4371392, sha256: SHA, source: "release", parts: 1 }, end()); }
    if (args[0] === "helper" && args[1] === "remove") { there = false; return jl(header("helper"), { type: "removed", machine: "box", path: "~/.local/share/flockbook/helper" }, end()); }
    if (args[0] === "feed" && !there) return jl(header("feed"), fake.why ?? missing(), end(false));
    if (args[0] === "feed") return jl(header("feed"), { type: "log", agent: "claude", path: "/l.jsonl", start: 0, size: 1, cursor: "c1" }, { type: "splice", at: 0, count: 1 }, { type: "row", id: "u1", k: "user", text: "hello from box", ts: 1 }, end());
    return jl(header(args[0]), end());
  };
});

const open = async () => { const cx = fakeCx(); const v = new ChatView(cx); await cx.settle(); v.width = 900; return { cx, v }; };

test("machineOf: only one level, m/tN, with or without hc:", () => {
  assert.equal(machineOf("box/t7"), "box");
  assert.equal(machineOf("hc:box/t7"), "box");
  assert.equal(machineOf("t7"), null);
  assert.equal(machineOf("a/b/t7"), null);
  assert.equal(machineOf(null), null);
});

test("no helper there: a card says which machine, where, how big and the checksum; nothing is put there yet", async () => {
  const { cx, v } = await open();
  const el = v.render(cx);
  const text = textOf(el);
  assert.match(text, /Install the Flockbook helper on box\?/);
  assert.match(text, /~\/\.local\/share\/flockbook\/helper\/0\.1\.0\/flockbook-modfeed/);
  assert.match(text, /4\.4 MB/);
  assert.match(text, /x86_64-unknown-linux-musl/);
  assert.match(text, new RegExp(SHA), "the full sha256 is on the card");
  assert.doesNotMatch(text, /development source/i);
  assert.ok(byId(el, "helper-install") && byId(el, "helper-cancel"));
  assert.ok(!calls.some((a) => a[0] === "helper"), "nothing goes to the machine before a press");
  for (const t of cx.timers) await t.f(cx);
  await cx.settle();
  assert.ok(!calls.some((a) => a[0] === "helper"), "a timer never installs");
});

test("Install puts the helper there with the card's checksum, then the chat loads", async () => {
  const { cx, v } = await open();
  byId(v.render(cx), "helper-install")._h.on_click({}, cx);
  assert.match(textOf(v.render(cx)), /Installing on box/);
  await cx.settle();
  assert.deepEqual(calls.find((a) => a[0] === "helper"), ["helper", "install", "--machine", "box", "--sha256", SHA, "--cache", "/home/test/.config/herdr-app/flockbook/cache.json"]);
  await v.tick(cx); await cx.settle();
  v.render(cx);
  assert.equal(v.feed().rows[0]?.text, "hello from box");
});

test("Cancel installs nothing; the card can come back", async () => {
  const { cx, v } = await open();
  byId(v.render(cx), "helper-cancel")._h.on_click({}, cx);
  await cx.settle();
  const el = v.render(cx);
  assert.match(textOf(el), /Not installed/);
  assert.equal(byId(el, "helper-install"), null);
  assert.ok(!calls.some((a) => a[0] === "helper"));
  byId(el, "helper-again")._h.on_click({}, cx);
  assert.ok(byId(v.render(cx), "helper-install"));
});

test("a failed install shows why and installs nothing more", async () => {
  const { cx, v } = await open();
  fake.exec = (_c, args) => { calls.push(args); return args[0] === "helper"
    ? jl(header("helper"), { type: "error", code: "checksum", message: "The file is not the one you said yes to." }, end(false))
    : jl(header("feed"), missing(), end(false)); };
  byId(v.render(cx), "helper-install")._h.on_click({}, cx);
  await cx.settle();
  const text = textOf(v.render(cx));
  assert.match(text, /Nothing was installed/);
  assert.match(text, /not the one you said yes to/);
});

test("a development source is marked on the card", async () => {
  fake.why = missing(install({ source: "dev" }));
  const { cx, v } = await open();
  assert.match(textOf(v.render(cx)), /Development source/);
});

test("another version there: the card says it replaces it", async () => {
  fake.why = { type: "error", code: "remote_helper_outdated", message: "The helper on box is 0.0.9.", remote: { protocol: 1, sidecar: "0.0.9" }, install: install() };
  const { cx, v } = await open();
  const text = textOf(v.render(cx));
  assert.match(text, /another version \(0\.0\.9\)/);
  assert.match(text, /Install the Flockbook helper on box\?/);
});

test("no helper for that platform, Windows, a bad address, a failed call: clear words, no card", async () => {
  const show = async (why) => { fake.why = why; const { cx, v } = await open(); const el = v.render(cx); return { text: textOf(el), card: byId(el, "helper-install") }; };
  let r = await show(missing(null));
  assert.match(r.text, /This Flockbook has no helper for that machine/); assert.equal(r.card, null);
  r = await show({ type: "error", code: "remote_helper_unsupported", message: "box runs Windows" });
  assert.match(r.text, /cannot run on box yet/); assert.equal(r.card, null);
  r = await show({ type: "error", code: "bad_terminal", message: "x" });
  assert.match(r.text, /not a terminal address/i);
  r = await show({ type: "error", code: "remote_failed", message: "timed out" });
  assert.match(r.text, /did not answer/);
});

test("a remote tab's menu offers Remove helper; the chat asks before it removes", async () => {
  const t = { id: "tb-box/t7", index: 1, name: "1", active: true, panes: 1 };
  const items = tabItems(t, { agent: fake.agents[0], list: [t] }).filter(Boolean);
  const item = items.find((i) => i.label === "Remove helper from box…");
  assert.ok(item, "remote tab has the item");
  assert.ok(!tabItems(t, { agent: agent("t1", "idle"), list: [t] }).filter(Boolean).some((i) => /Remove helper/.test(i.label)), "a local tab has not");
  there = true;
  const { cx, v } = await open();
  act(item);
  assert.equal(fake.shared[KEY_REMOVE], "box");
  await v.tick(cx); await cx.settle();
  let el = v.render(cx);
  assert.match(textOf(el), /Remove the Flockbook helper from box\?/);
  assert.ok(!calls.some((a) => a[0] === "helper"), "nothing removed before a press");
  byId(el, "helper-remove")._h.on_click({}, cx); await cx.settle();
  assert.deepEqual(calls.find((a) => a[0] === "helper"), ["helper", "remove", "--machine", "box", "--cache", "/home/test/.config/herdr-app/flockbook/cache.json"]);
  assert.equal(fake.shared[KEY_REMOVE], null);
  // Cancel: nothing is removed.
  calls.length = 0; act(item); await v.tick(cx); await cx.settle();
  byId(v.render(cx), "helper-keep")._h.on_click({}, cx); await cx.settle();
  assert.equal(fake.shared[KEY_REMOVE], null);
  assert.ok(!calls.some((a) => a[0] === "helper"));
});
