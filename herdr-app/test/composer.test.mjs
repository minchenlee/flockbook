import { test, beforeEach } from "node:test";
import assert from "node:assert/strict";
import { fake, log, reset } from "./stub/herdr.mjs";
import { textOf, byId } from "./stub/ui.mjs";
import { fakeCx } from "./fake-cx.mjs";
import { agent, space } from "./fixtures.mjs";
import { Composer, tagsFor, shortPath, quoted, withFiles } from "../mods/flockbook/composer.js";
import { ChatView } from "../mods/flockbook/chat.js";

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
  assert.deepEqual(bound, [{ keystroke: "secondary-v", action: "flockbook_paste", context: "FlockbookComposer > Input" }]);
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
