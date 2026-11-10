import { test, beforeEach } from "node:test";
import assert from "node:assert/strict";
import { fake, log, reset } from "./stub/herdr.mjs";
import { textOf, byId } from "./stub/ui.mjs";
import { fakeCx } from "./fake-cx.mjs";
import { agent, space } from "./fixtures.mjs";
import { Composer, tagsFor, shortPath, quoted, withFiles, slashQuery, matchCommands, screenNote, argItems, matchArgs } from "../mods/flockbook/composer.js";
import { ChatView } from "../mods/flockbook/chat.js";
import { forgetCommands } from "../mods/flockbook/sidecar.js";

const jl = (...lines) => ({ code: 0, stdout: lines.map((l) => JSON.stringify(l)).join("\n") + "\n", stderr: "" });
const header = (command) => ({ type: "header", protocol: 1, sidecar: "0.1.0", command });
const end = { type: "end", ok: true };
const statusLine = { type: "status", agent: "claude", source: "statusline", tags: [
  { kind: "model", text: "Opus 5.5", warn: false }, { kind: "effort", text: "high", warn: false },
  { kind: "context", text: "ctx 84%", percent: 84, warn: true }, { kind: "cost", text: "$0.42 est.", warn: false }] };

let sent;
beforeEach(() => {
  reset(); sent = [];
  fake.spaces = [space()];
  fake.agents = [agent("t1", "idle", { tab_name: "api" }), agent("t2", "idle")];
  fake.panes = [{ terminal: "t1", cwd: "/home/test/w/app", branch: "feat/login", status: "idle" }];
  fake.exec = (cmd, args) => {
    if (args[0] === "status") return jl(header("status"), statusLine, end);
    if (args[0] === "send") { sent.push(args); return jl(header("send"), { type: "done", result: fake.sendResult ?? "sent", message: fake.sendMessage ?? null }, end); }
    return jl(header(args[0]), end);
  };
});

test("shortPath keeps the folder's last part; home is ~", () => {
  assert.equal(shortPath("/home/test/w/app"), "app");
  assert.equal(shortPath("/a/b/c/d/e/"), "e");
  assert.equal(shortPath("/home/test"), "~");
});

test("files are named in the text, quoted when needed", () => {
  assert.equal(quoted("/tmp/a.png"), "/tmp/a.png");
  assert.equal(quoted("/tmp/my shot.png"), "'/tmp/my shot.png'");
  assert.equal(quoted("/tmp/it's.png"), "'/tmp/it'\\''s.png'");
  assert.equal(withFiles("", ["/tmp/a b"]), "'/tmp/a b'");
  assert.equal(withFiles("look", []), "look");
});

test("tags: the sidecar's, then folder, branch and agent name", () => {
  const tags = tagsFor(fake.agents[0], statusLine);
  assert.deepEqual(tags.map((t) => t.text), ["Opus 5.5", "high", "ctx 84%", "$0.42 est.", "app", "feat/login", "api"]);
  const dup = { tags: [...statusLine.tags, { kind: "own", text: "Opus 5.5 · high" }, { kind: "own", text: "…/app" }, { kind: "model", text: "api" }] };
  assert.deepEqual(tagsFor(fake.agents[0], dup).map((t) => t.text), ["Opus 5.5", "high", "ctx 84%", "$0.42 est.", "api", "app", "feat/login"],
    "the sidecar's own tags are left out, and a text shows once");
  assert.equal(tags[2].warn, true);
  assert.deepEqual(tagsFor(fake.agents[1], null).map((t) => t.kind), ["agent"]);
});

test("status is read for the shown agent and drawn as tags", async () => {
  const cx = fakeCx(); const c = new Composer(cx);
  c.box(fake.agents[0]);
  await c.pollStatus(cx);
  const el = c.render(fake.agents[0], new Proxy({}, { get: (_t, k) => `#${k}` }));
  assert.match(textOf(byId(el, "tags")), /Opus 5\.5.*ctx 84%.*feat\/login.*api/);
  assert.deepEqual(log.exec[0][1].slice(0, 3), ["status", "--terminal", "t1"]);
});

test("Enter sends the text and files through the sidecar, then empties the box", async () => {
  const cx = fakeCx(); const c = new Composer(cx);
  const b = c.box(fake.agents[0]);
  b.input.set_value("Fix it\n"); c.addFiles(["/tmp/a.png", "/tmp/a.png", "/tmp/b.txt"], cx);
  b.input._h.submit({}, cx); await cx.settle();
  assert.deepEqual(sent[0], ["send", "--terminal", "t1", "--text", "Fix it /tmp/a.png /tmp/b.txt", "--file", "/tmp/a.png", "--file", "/tmp/b.txt", "--home", "/home/test/.config/herdr-app"]);
  assert.equal(b.input.value(), ""); assert.deepEqual(b.files, []);
});

test("a blank box sends nothing; a failed send puts the text back with the reason", async () => {
  const cx = fakeCx(); const c = new Composer(cx);
  const b = c.box(fake.agents[0]);
  b.input.set_value("  \n"); await c.submit(cx);
  assert.equal(sent.length, 0);
  fake.sendResult = "failed"; fake.sendMessage = "The agent's terminal is gone.";
  b.input.set_value("hello"); await c.submit(cx);
  assert.equal(b.input.value(), "hello");
  assert.match(textOf(byId(c.render(fake.agents[0], new Proxy({}, { get: (_t, k) => `#${k}` })), "send-note")), /terminal is gone/);
  // Another agent in the pane: not sent either; the text comes back with the reason.
  fake.sendResult = "changed"; fake.sendMessage = "another agent is in this pane now; nothing was sent";
  b.input.set_value("again"); await c.submit(cx);
  assert.equal(b.input.value(), "again");
  assert.match(textOf(byId(c.render(fake.agents[0], new Proxy({}, { get: (_t, k) => `#${k}` })), "send-note")), /Another agent is in this pane now/);
  // An agent Flockbook cannot tell apart: not sent, the text comes back, the sidecar's reason as it is.
  fake.sendResult = "no-identity"; fake.sendMessage = "Flockbook cannot tell this agent from another program in this terminal, so it sends nothing. Use the terminal.";
  b.input.set_value("third"); await c.submit(cx);
  assert.equal(b.input.value(), "third");
  assert.match(textOf(byId(c.render(fake.agents[0], new Proxy({}, { get: (_t, k) => `#${k}` })), "send-note")), /cannot tell this agent/);
  // Text already in the agent's own box: not sent, the text stays, the helper says to clear the box.
  fake.sendResult = "box_not_empty"; fake.sendMessage = "The agent's box has text in it. Clear it in the terminal first. Nothing was sent.";
  b.input.set_value("/status"); await c.submit(cx);
  assert.equal(b.input.value(), "/status");
  assert.match(textOf(byId(c.render(fake.agents[0], new Proxy({}, { get: (_t, k) => `#${k}` })), "send-note")), /Clear it in the terminal first/);
});

test("each agent keeps its own draft", () => {
  const cx = fakeCx(); const c = new Composer(cx);
  c.box(fake.agents[0]).input.set_value("for api");
  c.box(fake.agents[1]).input.set_value("for t2");
  assert.equal(c.box(fake.agents[0]).input.value(), "for api");
});

test("the chat shows the composer, and a drop on it attaches files", async () => {
  fake.exec = (cmd, args) => args[0] === "feed"
    ? jl(header("feed"), { type: "log", agent: "claude", path: "/l.jsonl", start: 0, size: 1, cursor: "c" }, { type: "splice", at: 0, count: 1 }, { type: "row", id: "u1", k: "user", text: "hi" }, end)
    : jl(header(args[0]), end);
  const cx = fakeCx(); const v = new ChatView(cx); await cx.settle();
  let el = v.render(cx);
  assert.ok(byId(el, "composer"));
  el._h.on_drop({ paths: ["/tmp/shot.png"] }, cx);
  el = v.render(cx);
  assert.match(textOf(byId(el, "file-0")), /shot\.png/);
});

test("the + file dialog (osascript) shows on macOS only; drop still attaches on Linux", async () => {
  fake.exec = (cmd, args) => args[0] === "feed"
    ? jl(header("feed"), { type: "log", agent: "claude", path: "/l.jsonl", start: 0, size: 1, cursor: "c" }, { type: "splice", at: 0, count: 1 }, { type: "row", id: "u1", k: "user", text: "hi" }, end)
    : jl(header(args[0]), end);
  const cx = fakeCx(); const v = new ChatView(cx); await cx.settle();
  assert.ok(byId(v.render(cx), "attach"));
  const was = fake.window.platform;
  fake.window.platform = "linux";
  try {
    let el = v.render(cx);
    assert.ok(!byId(el, "attach"));
    el._h.on_drop({ paths: ["/tmp/shot.png"] }, cx);
    assert.match(textOf(byId(v.render(cx), "file-0")), /shot\.png/);
  } finally { fake.window.platform = was; }
});

// ⌘V in the box: the box's own paste runs before any key handler, so the mod binds the chord under its own
// key context and asks the sidecar first: files, else plain text (the box types it), else an image.
const PASTE_DIR = "/home/test/.config/herdr-app/flockbook/paste";
function pasteRig(answer) {
  const asked = [];
  fake.exec = (cmd, args) => {
    if (args[0] === "paste-image") { asked.push(args); return answer ? jl(header("paste-image"), ...answer, end) : { code: 1, stdout: "", stderr: "boom" }; }
    return jl(header(args[0]), end);
  };
  const cx = fakeCx(); const v = new ChatView(cx);
  const typed = [];
  v.composer.input.replace = (x) => typed.push(x);
  const press = async (clip) => {
    cx.read_from_clipboard = () => clip;
    const el = byId(v.render(cx), "composer");
    el._h.on_action(null, cx); await cx.settle();
    // An element stub answers every property, `then` too, so it goes back wrapped (or `await` never ends).
    return { el: byId(v.render(cx), "composer") };
  };
  return { asked, cx, v, typed, press };
}
const chips = (el) => [0, 1, 2].map((i) => byId(el, `file-${i}`)).filter(Boolean).map((c) => textOf(c).replace(/\s*×$/, ""));

test("paste: the mod binds ⌘V (Ctrl+V on Linux) for its own box only, once", async () => {
  const cx = fakeCx(); const bound = [];
  cx.bind_keys = (b) => { bound.push(...b); return b.length; };
  const v = new ChatView(cx); await cx.settle(); new ChatView(cx);
  assert.deepEqual(bound.filter((b) => b.action === "flockbook_paste"), [{ keystroke: "secondary-v", action: "flockbook_paste", context: "FlockbookComposer > Input" }]);
  // The "/" list's keys, once, under the context its wrapper has only while the list shows.
  assert.deepEqual(bound.filter((x) => x.action.startsWith("flockbook_slash")).map((x) => [x.keystroke, x.context]),
    [["up", "FlockbookSlash > Input"], ["down", "FlockbookSlash > Input"], ["tab", "FlockbookSlash > Input"], ["escape", "FlockbookSlash > Input"]]);
  const el = byId(v.render(cx), "composer");
  assert.equal(el._a.key_context, "FlockbookComposer");
  assert.equal(typeof el._h.on_action, "function");
});

test("paste: a screenshot (image, no text) is attached, nothing typed", async () => {
  const r = pasteRig([{ type: "image", path: `${PASTE_DIR}/paste-1.png` }]);
  const { el } = await r.press(undefined);
  assert.deepEqual(r.asked[0].slice(0, 3), ["paste-image", "--dir", PASTE_DIR]);
  assert.deepEqual(chips(el), ["paste-1.png"]);
  assert.deepEqual(r.typed, []);
});

test("paste: Finder files are attached, not their names typed", async () => {
  const r = pasteRig([{ type: "files", paths: ["/home/you/Shot 1.png", "/home/you/notes.txt"] }, { type: "image", path: null }]);
  const { el } = await r.press("Shot 1.png\rnotes.txt");
  assert.deepEqual(chips(el), ["Shot 1.png", "notes.txt"]);
  assert.deepEqual(r.typed, []);
});

test("paste: a browser's Copy Image (image and HTML, no plain text) is attached", async () => {
  const r = pasteRig([{ type: "image", path: `${PASTE_DIR}/paste-2.png` }]);
  const { el } = await r.press(undefined);
  assert.equal(chips(el).length, 1);
  assert.deepEqual(r.typed, []);
});

test("paste: plain text (alone, or with a picture of spreadsheet cells) is typed at the cursor; a missing sidecar still pastes text", async () => {
  const r = pasteRig([{ type: "image", path: null }]);
  let { el } = await r.press("hello");
  assert.deepEqual(r.typed, ["hello"]);
  assert.equal(chips(el).length, 0);
  const broken = pasteRig(null);
  ({ el } = await broken.press("still text"));
  assert.deepEqual(broken.typed, ["still text"]);
  assert.ok(!byId(el, "send-note"), "no error for a text paste");
});

test("paste: nothing to paste and a broken sidecar says why", async () => {
  const r = pasteRig(null);
  const { el } = await r.press(undefined);
  assert.match(textOf(byId(el, "send-note")), /sidecar/);
  assert.deepEqual(r.typed, []);
});

test("paste: when the box's own paste finds no text (no binding), the image is still attached", async () => {
  const r = pasteRig([{ type: "image", path: `${PASTE_DIR}/paste-3.png` }]);
  r.v.render(r.cx);
  r.v.composer.input._h.paste_other({}, r.cx); await r.cx.settle();
  assert.equal(chips(byId(r.v.render(r.cx), "composer")).length, 1);
});

test("Shift+Enter leaves the new line to the box and sends nothing; Enter sends", async () => {
  // The box puts the new line in itself (at the cursor, over a selection) and then reports a submit with
  // shift. The handler must not add a second one.
  const cx = fakeCx(); const c = new Composer(cx);
  const b = c.box(fake.agents[0]);
  b.input.set_value("first line");
  b.input._h.submit({ shift: true }, cx); await cx.settle();
  assert.equal(sent.length, 0, "Shift+Enter does not send");
  assert.equal(b.input.value(), "first line", "the handler adds no text");
  b.input.set_value("first line\nsecond line");
  b.input._h.submit({}, cx); await cx.settle();
  assert.equal(sent.length, 1, "Enter sends");
  assert.equal(sent[0][sent[0].indexOf("--text") + 1], "first line\nsecond line");
});

// Enter in the box puts a new line at the cursor (over a selection) before it reports the submit. In the
// middle of the text that line went out, and came back on a failed send. `type` is what the box does: set the
// value, then report the change (or not: the order is the box's).
function enterRig() {
  const cx = fakeCx(); const c = new Composer(cx);
  const b = c.box(fake.agents[0]);
  const type = (v, report = true) => { b.input.set_value(v); if (report) b.input._h.change({}, cx); };
  const text = () => sent[sent.length - 1][sent[sent.length - 1].indexOf("--text") + 1];
  return { cx, b, type, text };
}

test("Enter in the middle of the text sends it without the new line Enter put there", async () => {
  for (const reported of [true, false]) {
    const { cx, b, type, text } = enterRig();
    type("ab"); type("abcd");
    type("ab\ncd", reported);
    b.input._h.submit({}, cx); await cx.settle();
    assert.equal(text(), "abcd", `change reported: ${reported}`);
  }
});

test("Enter over a selection sends the text as it was", async () => {
  const { cx, b, type, text } = enterRig();
  type("abXcd"); type("ab\ncd");
  b.input._h.submit({}, cx); await cx.settle();
  assert.equal(text(), "abXcd");
});

test("a failed send puts the text back without Enter's new line", async () => {
  fake.sendResult = "failed"; fake.sendMessage = "Not sent.";
  const { cx, b, type } = enterRig();
  type("abcd"); type("ab\ncd");
  b.input._h.submit({}, cx); await cx.settle();
  assert.equal(b.input.value(), "abcd");
});

test("Shift+Enter's new lines stay; Enter at the end adds none", async () => {
  const { cx, b, type, text } = enterRig();
  type("ab"); type("ab\n"); b.input._h.submit({ shift: true }, cx);
  type("ab\ncd"); type("ab\ncd\n");
  b.input._h.submit({}, cx); await cx.settle();
  assert.equal(text(), "ab\ncd");
  // A typed new line in the middle, then Enter at the end.
  type("x"); type("x\n\ny"); type("x\n\ny\n", false);
  b.input._h.submit({}, cx); await cx.settle();
  assert.equal(text(), "x\n\ny");
});

test("send names the agent and the log the chat showed, so the sidecar can refuse another one", async () => {
  const cx = fakeCx(); const c = new Composer(cx);
  c.seen = (address) => address === "t1" ? { agent: "claude", path: "/logs/a.jsonl" } : null;
  const b = c.box(fake.agents[0]);
  b.input.set_value("Go on"); b.input._h.submit({}, cx); await cx.settle();
  assert.deepEqual(sent[0], ["send", "--terminal", "t1", "--text", "Go on", "--expect-kind", "claude", "--expect-path", "/logs/a.jsonl", "--home", "/home/test/.config/herdr-app"]);
});

test("the chat gives the composer what its feed showed", async () => {
  const cx = fakeCx(); const v = new ChatView(cx); await cx.settle();
  v.feeds.set("t9", { agent: "codex", path: "/l/x.jsonl" });
  assert.deepEqual(v.composer.seen("t9"), { agent: "codex", path: "/l/x.jsonl" });
});

// "/" commands (Claude Code only): the list above the box while the text is "/" and a name, no space.
const CMDS = [
  { name: "clear", description: "Start a new conversation", source: "builtin" },
  { name: "compact", description: "Summarise the conversation", source: "builtin" },
  { name: "context", description: "Show how full the context is", source: "builtin" },
  { name: "deploy", description: "Deploy the app", source: "project" },
];
const T = new Proxy({}, { get: (_t, k) => `#${k}` });
function slashRig({ hasCommands = true, list = CMDS, screens = [] } = {}) {
  const asked = [];
  forgetCommands();
  fake.exec = (cmd, args) => {
    if (args[0] === "version") return jl(header("version"), { type: "version", commands: ["version", "send", ...(hasCommands ? ["commands"] : [])] }, end);
    if (args[0] === "commands") { asked.push(args); return jl(header("commands"), { type: "commands", agent: "claude", commands: list, screens }, end); }
    if (args[0] === "send") { sent.push(args); return jl(header("send"), { type: "done", result: "sent" }, end); }
    return jl(header(args[0]), end);
  };
  const cx = fakeCx(); const bound = [];
  cx.bind_keys = (b) => { bound.push(...b); return b.length; };
  const c = new Composer(cx);
  const b = c.box(fake.agents[0]);
  const type = async (v) => { b.input.set_value(v); b.input._h.change({}, cx); await cx.settle(); };
  const view = () => c.render(fake.agents[0], T);
  const rows = () => { const l = byId(view(), "slash"); return l ? l.kids.map((k) => textOf(k)) : null; };
  const act = async (name) => { byId(view(), "slash-keys")._h[`on_action:${name}`](null, cx); await cx.settle(); };
  return { cx, c, b, type, view, rows, act, asked, bound };
}

test("slashQuery: the name after a leading /, while there is no space; matchCommands: by prefix, at most 8", () => {
  assert.equal(slashQuery("/"), "");
  assert.equal(slashQuery("/de"), "de");
  assert.equal(slashQuery("/kit:he"), "kit:he");
  for (const t of ["", "de", " /de", "/de ", "/deploy now", "/a\nb"]) assert.equal(slashQuery(t), null, JSON.stringify(t));
  assert.deepEqual(matchCommands(CMDS, "c").map((x) => x.name), ["clear", "compact", "context"]);
  assert.deepEqual(matchCommands(CMDS, "CO").map((x) => x.name), ["compact", "context"]);
  assert.deepEqual(matchCommands(CMDS, "zz"), []);
  const many = Array.from({ length: 20 }, (_v, i) => ({ name: `cmd${i}` }));
  assert.equal(matchCommands(many, "").length, 20, "all matches: the list scrolls");
});

test("typing / in a Claude Code chat shows its commands above the box, filtered as you type", async () => {
  const r = slashRig();
  assert.equal(r.rows(), null, "nothing before a /");
  await r.type("/");
  assert.equal(r.rows().length, 4);
  assert.match(r.rows()[3], /deploy.*Deploy the app/);
  await r.type("/co");
  assert.deepEqual(r.rows().map((x) => x.split(/\s/)[0]), ["/compact", "/context"]);
  await r.type("/co now");
  assert.equal(r.rows(), null, "a space closes it");
  assert.deepEqual(r.asked.map((a) => a.slice(0, 3)), [["commands", "--terminal", "t1"]], "the list is asked once");
  // The keys go to the list only while it shows: its key context is on then, and off without it.
  assert.equal(byId(r.view(), "slash-keys")._a.key_context, undefined);
  await r.type("/d");
  assert.equal(byId(r.view(), "slash-keys")._a.key_context, "FlockbookSlash");
});

test("up and down choose (stopping at the ends), Tab completes the full name and a space; Esc closes until the text changes", async () => {
  const r = slashRig();
  await r.type("/c");
  await r.act("flockbook_slash_up");
  await r.act("flockbook_slash_down");
  await r.act("flockbook_slash_complete");
  assert.equal(r.b.input.value(), "/compact ", "up on the first row stays there; down: the second");
  assert.equal(r.rows(), null);
  assert.equal(sent.length, 0, "completing sends nothing");
  await r.type("/c");
  for (let i = 0; i < 5; i++) await r.act("flockbook_slash_down");
  await r.act("flockbook_slash_complete");
  assert.equal(r.b.input.value(), "/context ", "down on the last row stays there (no wrap)");
  await r.type("/c");
  await r.act("flockbook_slash_close");
  assert.equal(r.rows(), null, "Esc closes it");
  await r.type("/cl");
  assert.equal(r.rows().length, 1, "typing opens it again");
});

test("Enter completes a part of a name; on a full name it sends that name", async () => {
  const r = slashRig();
  await r.type("/dep");
  r.b.input.set_value("/dep\n"); r.b.input._h.submit({}, r.cx); await r.cx.settle();
  assert.equal(sent.length, 0, "a part of a name is not sent (Claude Code's own menu would run its first match)");
  assert.equal(r.b.input.value(), "/deploy ");
  await r.type("/clear");
  r.b.input._h.submit({}, r.cx); await r.cx.settle();
  assert.equal(sent.length, 1);
  assert.equal(sent[0][sent[0].indexOf("--text") + 1], "/clear");
});

test("a list for any agent the helper lists commands for; none when its list is empty or the helper has no commands", async () => {
  const r = slashRig({ hasCommands: false });
  await r.type("/");
  assert.equal(r.rows(), null);
  assert.equal(r.asked.length, 0, "a helper without commands is not asked");
  const s = slashRig();
  fake.agents[0] = agent("t1", "idle", { tab_name: "api", app: "codex", program: { key: "codex", name: "Codex" } });
  s.c.box(fake.agents[0]);
  await s.type("/");
  assert.equal(byId(s.c.render(fake.agents[0], T), "slash").kids.length, 4, "a Codex chat shows what the helper lists");
  assert.equal(s.asked.length, 1);
  const e = slashRig({ list: [] });
  await e.type("/");
  assert.equal(e.rows(), null, "an empty list (an agent the helper has no commands for) shows nothing");
  await e.type("/c");
  assert.equal(e.asked.length, 1, "and is not asked again at once");
});

// More matches than rows: a window of 8 follows the pick; the wheel moves it.
const MANY = Array.from({ length: 20 }, (_v, i) => ({ name: `cmd${String(i).padStart(2, "0")}`, description: `Command ${i}`, source: "user" }));
const shown = (r) => r.rows().filter((x) => x.startsWith("/")).map((x) => x.split(/\s/)[0]);

test("20 matches: 8 rows show; down past the last row moves the window; the count says there are more", async () => {
  const r = slashRig({ list: MANY });
  await r.type("/cmd");
  assert.deepEqual(shown(r), MANY.slice(0, 8).map((c) => `/${c.name}`));
  assert.match(textOf(byId(r.view(), "slash-more")), /1–8 of 20/);
  for (let i = 0; i < 10; i++) await r.act("flockbook_slash_down");
  assert.deepEqual(shown(r), MANY.slice(3, 11).map((c) => `/${c.name}`), "pick 10 is the window's last row: rows 3 to 10");
  assert.match(textOf(byId(r.view(), "slash-more")), /4–11 of 20/);
  for (let i = 0; i < 20; i++) await r.act("flockbook_slash_down");
  assert.deepEqual(shown(r).at(-1), "/cmd19", "down stops at the last match");
  for (let i = 0; i < 19; i++) await r.act("flockbook_slash_up");
  assert.deepEqual(shown(r), MANY.slice(0, 8).map((c) => `/${c.name}`), "up to the first: the window follows back");
  for (let i = 0; i < 12; i++) await r.act("flockbook_slash_down");
  await r.type("/cmd1");
  assert.deepEqual(shown(r), MANY.slice(10, 18).map((c) => `/${c.name}`), "a new filter starts at its first match");
  await r.act("flockbook_slash_complete");
  assert.equal(r.b.input.value(), "/cmd10 ");
});

test("the wheel moves the window a row per 30 px; the pick stays on a shown row", async () => {
  const r = slashRig({ list: MANY });
  await r.type("/cmd");
  const wheel = async (dy) => { byId(r.view(), "slash")._h.on_scroll_wheel({ delta: { x: 0, y: dy }, touch_phase: "moved" }, r.cx); await r.cx.settle(); };
  await wheel(-90);
  assert.deepEqual(shown(r), MANY.slice(3, 11).map((c) => `/${c.name}`), "down 90 px: three rows");
  await r.act("flockbook_slash_complete");
  assert.equal(r.b.input.value(), "/cmd03 ", "the pick moved into the window");
  await r.type("/cmd");
  await wheel(-15); await wheel(-15);
  assert.equal(shown(r)[0], "/cmd01", "small trackpad steps add up");
  await wheel(+1000);
  assert.equal(shown(r)[0], "/cmd00", "up stops at the top");
  await wheel(-10000);
  assert.equal(shown(r).at(-1), "/cmd19", "down stops at the bottom");
});

const BUILT = [
  { name: "clear", description: "Start a new conversation", source: "builtin" },
  { name: "effort", description: "Set effort level", source: "builtin", hint: "low|medium|high|xhigh|max", args: ["low", "medium", "high", "xhigh", "max"], screen: true },
  { name: "model", description: "Set the AI model", source: "builtin", hint: "<model>", args: ["opus", "sonnet", "haiku", "fable", "default"], screen: true },
];

test("a built-in shows its argument hint, and \"needs a value\" when sent bare it opens a screen in the terminal", async () => {
  const r = slashRig({ list: BUILT });
  await r.type("/");
  const rows = r.rows();
  assert.match(rows[2], /\/model\s*<model>.*Set the AI model.*needs a value/);
  assert.doesNotMatch(rows[0], /needs a value/);
});

test("screenNote: a screen command alone, with or without a value it takes; anything else is null", () => {
  const screens = ["status", "config"];
  assert.equal(screenNote(BUILT, screens, "/model"), "/model opens a menu in the terminal. Add a value, like /model opus.");
  assert.equal(screenNote(BUILT, screens, "  /status\n"), "/status opens a screen in the terminal. Use it there.");
  const hintOnly = [{ name: "resume", hint: "[session]", screen: true }];
  assert.equal(screenNote(hintOnly, [], "/resume"), "/resume opens a menu in the terminal. Add a value, like /resume [session].");
  for (const t of ["/model sonnet", "/status now", "/clear", "/mode", "model", "hi /model", "/", ""]) assert.equal(screenNote(BUILT, screens, t), null, JSON.stringify(t));
  assert.equal(screenNote(undefined, undefined, "/model"), null, "no list yet: the helper's send checks");
});

test("Enter on a bare screen command sends nothing, keeps the text and says why; with a value it goes out", async () => {
  const r = slashRig({ list: [...BUILT, { name: "statusline", description: "Set up the status line", source: "user" }], screens: ["status"] });
  const before = sent.length;
  const enter = async (v) => { await r.type(v); r.b.input.set_value(`${v}\n`); r.b.input._h.submit({}, r.cx); await r.cx.settle(); };
  await enter("/model");
  assert.equal(sent.length, before, "nothing sent");
  assert.equal(r.b.input.value(), "/model", "the text stays for a value");
  assert.match(textOf(byId(r.view(), "send-note")), /\/model opens a menu in the terminal\. Add a value, like \/model opus\./);
  await enter("/status");
  assert.equal(sent.length, before, "a left-out screen is not completed to another name, and not sent");
  assert.equal(r.b.input.value(), "/status");
  assert.match(textOf(byId(r.view(), "send-note")), /\/status opens a screen in the terminal\. Use it there\./);
  await enter("/model sonnet");
  assert.equal(sent.at(-1)[sent.at(-1).indexOf("--text") + 1], "/model sonnet");
  await enter("/clear");
  assert.equal(sent.at(-1)[sent.at(-1).indexOf("--text") + 1], "/clear");
});

test("after \"/model \" the list offers the model names; Tab or Enter completes one, Enter on a whole one sends it", async () => {
  const r = slashRig({ list: BUILT });
  await r.type("/mo");
  await r.act("flockbook_slash_complete");
  assert.equal(r.b.input.value(), "/model ");
  await r.type("/model ");
  assert.deepEqual(shown(r).length, 0, "values are not commands");
  assert.deepEqual(r.rows().map((x) => x.split(/\s/)[0]), ["opus", "sonnet", "haiku", "fable", "default"]);
  await r.type("/model s");
  assert.deepEqual(r.rows().map((x) => x.split(/\s/)[0]), ["opus", "sonnet"], "any part of a value");
  await r.type("/model so");
  assert.deepEqual(r.rows().map((x) => x.split(/\s/)[0]), ["sonnet"]);
  await r.act("flockbook_slash_complete");
  assert.equal(r.b.input.value(), "/model sonnet");
  r.b.input._h.submit({}, r.cx); await r.cx.settle();
  assert.equal(sent.at(-1)[sent.at(-1).indexOf("--text") + 1], "/model sonnet");
  await r.type("/effort x");
  r.b.input.set_value("/effort x\n"); r.b.input._h.submit({}, r.cx); await r.cx.settle();
  assert.equal(r.b.input.value(), "/effort xhigh", "Enter on a part of a value completes it");
  await r.type("/clear x");
  assert.equal(r.rows(), null, "a command without values: no list after its space");
});

// The helper's /model lists: { value, label } values (plain strings still work).
const LABELED = [
  { name: "model", description: "Set the AI model", source: "builtin", hint: "<model>", screen: true,
    args: [{ value: "claude-opus-5-5", label: "Opus 5.5" }, { value: "claude-sonnet-5-5", label: "Sonnet 5.5" }, { value: "sonnet", label: "The latest Sonnet" }] },
];

test("argItems and matchArgs: labeled or plain values, found by a part of the value or the label", () => {
  assert.deepEqual(argItems(["low", { value: "x", label: "X one" }, { value: "" }, null].filter((v) => v !== null)),
    [{ name: "low" }, { name: "x", description: "X one" }]);
  const items = argItems(LABELED[0].args);
  assert.deepEqual(matchArgs(items, "SONNET").map((x) => x.name), ["claude-sonnet-5-5", "sonnet"]);
  assert.deepEqual(matchArgs(items, "5.5").map((x) => x.name), ["claude-opus-5-5", "claude-sonnet-5-5"], "by the label");
  assert.deepEqual(matchArgs(items, "").length, 3);
});

test("/model lists full ids with their labels; a part completes; the note's example is the first value", async () => {
  const r = slashRig({ list: LABELED });
  await r.type("/model ");
  const rows = r.rows();
  assert.match(rows[0], /^claude-opus-5-5\s*Opus 5\.5/);
  await r.type("/model opus 5");
  assert.equal(r.rows(), null, "a space ends the value");
  await r.type("/model op");
  assert.deepEqual(r.rows().map((x) => x.split(/\s/)[0]), ["claude-opus-5-5"]);
  r.b.input.set_value("/model op\n"); r.b.input._h.submit({}, r.cx); await r.cx.settle();
  assert.equal(r.b.input.value(), "/model claude-opus-5-5", "Enter on a part completes it");
  r.b.input._h.submit({}, r.cx); await r.cx.settle();
  assert.equal(sent.at(-1)[sent.at(-1).indexOf("--text") + 1], "/model claude-opus-5-5");
  assert.equal(screenNote(LABELED, [], "/model"), "/model opens a menu in the terminal. Add a value, like /model claude-opus-5-5.");
});


test("the status tags row scrolls sideways with no scrollbar", async () => {
  const cx = fakeCx(); const c = new Composer(cx);
  c.box(fake.agents[0]);
  await c.pollStatus(cx);
  const row = byId(c.render(fake.agents[0], new Proxy({}, { get: (_t, k) => `#${k}` })), "tags");
  assert.ok("overflow_x_scroll" in row._a && !("overflow_x_scrollbar" in row._a));
});
