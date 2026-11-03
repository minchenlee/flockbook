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
  assert.deepEqual(sent[0], ["send", "--terminal", "t1", "--text", "Fix it /tmp/a.png /tmp/b.txt", "--file", "/tmp/a.png", "--file", "/tmp/b.txt"]);
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
