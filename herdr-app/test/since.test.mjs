import { test, beforeEach } from "node:test";
import assert from "node:assert/strict";
import { fake, log, reset } from "./stub/herdr.mjs";
import { textOf, byId, calls } from "./stub/ui.mjs";
import { fakeCx } from "./fake-cx.mjs";
import { agent, space } from "./fixtures.mjs";
import { ChatView } from "../mods/flockbook/chat.js";
import { forgetCommands } from "../mods/flockbook/sidecar.js";
import { KEY_GLANCE, KEY_MARKS, forgetGlance } from "../mods/flockbook/glance.js";
import { span, words, about, arrive, sinceLeft, countLine, AWAY_MS } from "../mods/flockbook/since.js";

const MIN = 60000;
const jl = (...lines) => ({ code: 0, stdout: lines.map((l) => JSON.stringify(l)).join("\n") + "\n", stderr: "" });
const head = (cmd) => ({ type: "header", protocol: 1, sidecar: "0.1.0", cmd });
const end = { type: "end", ok: true };
const gline = (extra = {}) => ({ type: "glance", terminal: "t1", agent: "claude", path: "/logs/s.jsonl", id: "a3", edited: 5, commands: 2, tests: "passed", test: "pytest", ...extra });
// A helper with `feed` and `glance`: `rows` and `glance` are what it reads from the log now.
const helper = (rows, glance) => (_cmd, args) => {
  if (args[0] === "version") return jl(head("version"), { type: "version", commands: ["version", "feed", "glance"] }, end);
  if (args[0] === "glance") return jl(head("glance"), glance(), end);
  if (args[0] === "feed") return jl(head("feed"), { type: "log", agent: "claude", path: "/logs/s.jsonl", start: 0, size: 10, cursor: "c1" }, { type: "splice", at: 0, count: rows.length }, ...rows, end);
  return jl(head(args[0]), end);
};

beforeEach(() => {
  reset(); calls.length = 0; forgetGlance(); forgetCommands();
  fake.spaces = [space()];
  fake.agents = [agent("t1", "done")];
});

test("since: the words of the counts", () => {
  assert.deepEqual([span(47 * MIN), span(MIN), span(3 * 60 * MIN), span(50 * 60 * MIN)], ["47 minutes", "1 minute", "3 hours", "2 days"]);
  assert.equal(words("Two words. And `three` more — yes"), 6);
  assert.equal(words("測試通過 ok"), 5, "each CJK character counts as a word");
  assert.deepEqual([about(7), about(64), about(1912), about(23456)], ["10", "60", "1,900", "23,000"]);
});

test("since: a card only after 5 minutes away, and only when the agent wrote more", () => {
  const now = 100 * MIN;
  assert.equal(arrive("t1", null, gline(), now), null, "a chat never opened before");
  assert.equal(arrive("t1", { id: "a3", at: 0 }, gline(), now), null, "nothing new");
  assert.equal(arrive("t1", { id: "a1", at: now - AWAY_MS + 1 }, gline(), now), null, "away too short");
  assert.deepEqual(arrive("t1", { id: "a1", at: now - 47 * MIN }, gline(), now), { address: "t1", from: now - 47 * MIN, back: now });
});

test("since: counts the agent's messages that came while away; names the owner's last message", () => {
  const rows = [
    { id: "u0", k: "user", text: "Old one", ts: 1 * MIN },
    { id: "a0", k: "assistant", text: "seen before", ts: 2 * MIN },
    { id: "n1", k: "user", from: "t2 docs-writer", text: "a note from an agent", ts: 9 * MIN },
    { id: "u1", k: "user", text: "\nFix the importer\nand the tests", ts: 10 * MIN },
    { id: "a1", k: "assistant", text: "one two three", ts: 20 * MIN },
    { id: "t1", k: "tools", chips: [], ts: 21 * MIN },
    { id: "a2", k: "assistant", text: "four five", ts: 30 * MIN },
    { id: "a3", k: "assistant", text: "after the owner came back", ts: 70 * MIN },
  ];
  const s = sinceLeft(rows, { address: "t1", from: 12 * MIN, back: 59 * MIN }, gline());
  assert.deepEqual([s.away, s.messages, s.words], ["47 minutes", 2, 5]);
  assert.equal(s.facts, "Edited 5 files · pytest passed after the last edit");
  assert.deepEqual([s.said.id, s.said.text], ["u1", "Fix the importer"]);
  assert.match(s.said.at, /^\d\d:\d\d$/);
  assert.equal(countLine(s), "You were away 47 minutes · 2 new messages · about 10 words");
  assert.match(countLine(sinceLeft([], { from: 0, back: 6 * MIN }, null)), /^You were away 6 minutes · no new message/);
});

test("chat: back after 47 minutes, the card says what came; Jump shows the owner's message; the turn is marked seen", async () => {
  const now = Date.now();
  const rows = [
    { type: "row", id: "u1", k: "user", text: "Fix the importer", ts: now - 60 * MIN },
    { type: "row", id: "a1", k: "assistant", text: "Looking.", ts: now - 59 * MIN },
    { type: "row", id: "a2", k: "assistant", text: "Found it. The fix is in.", ts: now - 30 * MIN },
    { type: "row", id: "a3", k: "assistant", text: "All tests pass now.", ts: now - 10 * MIN },
  ];
  let line = gline();
  fake.exec = helper(rows, () => line);
  fake.shared[KEY_MARKS] = { seen: { t1: { id: "a1", at: now - 47 * MIN, since: null } }, dismissed: {} };
  const cx = fakeCx();
  const v = new ChatView(cx); v.width = 900;
  await cx.settle();
  assert.equal(byId(v.render(cx), "since-card"), null, "no card before the helper's glance is read");
  await v.tick(cx); await cx.settle();
  await v.tick(cx); await cx.settle();
  let el = v.render(cx);
  const card = textOf(byId(el, "since-card"));
  assert.match(card, /Since you left/);
  assert.match(card, /You were away 47 minutes · 2 new messages · about 10 words/);
  assert.match(card, /From the log: Edited 5 files · pytest passed after the last edit/);
  assert.match(card, /You said at \d\d:\d\d: Fix the importer/);
  assert.doesNotMatch(card, /summary/i, "no model wrote anything here");
  assert.equal(fake.shared[KEY_MARKS].seen.t1.id, "a3", "the chat showed this turn");
  byId(el, "since-jump")._h.on_click({}, cx);
  assert.deepEqual(calls.at(-1), ["scroll_to_item", 0, "top"]);
  assert.equal(v.follow, false);
  // More from the agent while the chat is on screen: the card stays as it was, until Hide.
  line = gline({ id: "a4" });
  fake.shared["flockbook.glance.at"] = 0; forgetGlance();
  await v.tick(cx); await cx.settle(); await v.tick(cx); await cx.settle();
  assert.equal(fake.shared[KEY_MARKS].seen.t1.id, "a4");
  assert.match(textOf(byId(v.render(cx), "since-card")), /away 47 minutes/);
  byId(v.render(cx), "since-hide")._h.on_click({}, cx);
  assert.equal(byId(v.render(cx), "since-card"), null);
  // Back on this chat a moment later (from the Overview): no card.
  fake.shared["flockbook.screen"] = "overview"; await v.tick(cx);
  delete fake.shared["flockbook.screen"]; await v.tick(cx); await cx.settle();
  assert.equal(byId(v.render(cx), "since-card"), null);
});

test("chat: back from the Overview with an old glance line: the card waits for a new one", async () => {
  const now = Date.now();
  const rows = [
    { type: "row", id: "u1", k: "user", text: "Fix the importer", ts: now - 60 * MIN },
    { type: "row", id: "a1", k: "assistant", text: "Looking.", ts: now - 59 * MIN },
    { type: "row", id: "a3", k: "assistant", text: "All tests pass now.", ts: now - 10 * MIN },
  ];
  fake.exec = helper(rows, () => gline());
  // The chat was last on screen 47 minutes ago, at a1; the glance line kept since then still says a1, and
  // nothing polled while the Overview was on screen.
  fake.shared[KEY_GLANCE] = { on: true, by: { t1: gline({ id: "a1" }) } };
  fake.shared[KEY_MARKS] = { seen: { t1: { id: "a1", at: now - 47 * MIN, since: null } }, dismissed: {} };
  fake.shared["flockbook.screen"] = "overview";
  const cx = fakeCx();
  const v = new ChatView(cx); v.width = 900;
  await cx.settle(); await v.tick(cx); await cx.settle();
  delete fake.shared["flockbook.screen"];
  await v.tick(cx); await cx.settle();
  await v.tick(cx); await cx.settle();
  assert.match(textOf(byId(v.render(cx), "since-card")), /You were away 47 minutes/);
  assert.equal(fake.shared[KEY_MARKS].seen.t1.id, "a3");
});

test("chat: an agent with no glance id yet: the glance is asked once on the way in, not on every tick", async () => {
  fake.exec = helper([{ type: "row", id: "u1", k: "user", text: "hi", ts: 1 }], () => gline({ id: undefined }));
  const cx = fakeCx();
  const v = new ChatView(cx); v.width = 900;
  await cx.settle();
  for (let i = 0; i < 5; i++) { await v.tick(cx); await cx.settle(); }
  assert.equal(log.exec.filter((e) => e[1][0] === "glance").length, 1);
});

test("chat: a helper without glance shows no card and marks nothing", async () => {
  fake.exec = (_cmd, args) => (args[0] === "version" ? jl(head("version"), { type: "version", commands: ["version", "feed"] }, end)
    : jl(head("feed"), { type: "log", agent: "claude", path: "/l", start: 0, size: 1, cursor: "c" }, { type: "splice", at: 0, count: 1 }, { type: "row", id: "u1", k: "user", text: "hi", ts: 1 }, end));
  const cx = fakeCx();
  const v = new ChatView(cx); v.width = 900;
  await cx.settle(); await v.tick(cx); await cx.settle();
  assert.equal(byId(v.render(cx), "since-card"), null);
  assert.equal(fake.shared[KEY_MARKS]?.seen?.t1, undefined);
  assert.equal(fake.shared[KEY_GLANCE]?.on ?? false, false);
});
