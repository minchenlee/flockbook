import { test, beforeEach } from "node:test";
import assert from "node:assert/strict";
import { fake, log, reset } from "./stub/herdr.mjs";
import { textOf, byId } from "./stub/ui.mjs";
import { fakeCx } from "./fake-cx.mjs";
import { agent, space } from "./fixtures.mjs";
import { Cards, wanted, answersFor } from "../mods/flockbook/cards.js";
import { NeedsView } from "../mods/flockbook/needs.js";

const jl = (...lines) => ({ code: 0, stdout: lines.map((l) => JSON.stringify(l)).join("\n") + "\n", stderr: "" });
const header = (command) => ({ type: "header", protocol: 1, sidecar: "0.1.0", command });
const end = { type: "end", ok: true };
const T = new Proxy({}, { get: (_t, k) => `#${k}` });
const approval = { type: "card", kind: "approval", request: "toolu_1", title: "Bash command", command: "rm -rf build", tool: "Bash" };
const question = { type: "card", kind: "question", request: "toolu_q", unread: 0, question: { toolUseId: "toolu_q", state: "pending", delivery: "picker", questions: [
  { id: "a", header: "Color", text: "Pick a color?", multi: false, options: [{ label: "Red", detail: "warm", recommended: true }, { label: "Blue" }] },
  { id: "b", header: "Sizes", text: "Which sizes?", multi: true, options: [{ label: "S" }, { label: "M" }, { label: "L" }] }] } };

let cardNow, acts;
beforeEach(() => {
  reset(); acts = []; cardNow = approval;
  fake.spaces = [space()];
  fake.agents = [agent("t1", "blocked")];
  fake.exec = (_c, args) => {
    if (args[0] === "card") return jl(header("card"), cardNow ?? { type: "card" }, end);
    if (args[0] === "approve" || args[0] === "answer") { acts.push(args); const r = fake.result ?? "sent"; if (r === "sent") cardNow = null; return jl(header(args[0]), { type: "done", result: r, message: fake.message ?? null }, end); }
    return jl(header(args[0]), end);
  };
});

test("wanted: a blocked agent, or a log that ends with a pending call", () => {
  assert.equal(wanted(agent("t", "blocked"), []), true);
  assert.equal(wanted(agent("t", "working"), [{ k: "perm", perm: { state: "pending" } }]), true);
  assert.equal(wanted(agent("t", "working"), [{ k: "perm", perm: { state: "resolved" } }]), false);
  assert.equal(wanted(agent("t", "idle"), [{ k: "question", question: { state: "pending" } }, { k: "perm", perm: { state: "resolved" } }]), false);
  assert.equal(wanted(null, []), false);
});

test("answersFor: one per question, picks sorted, other text; null while one is empty", () => {
  const q = question.question;
  assert.equal(answersFor(q, new Map([[0, new Set([0])]]), new Map()), null);
  assert.deepEqual(answersFor(q, new Map([[0, new Set([0])], [1, new Set([2, 0])]]), new Map()), [{ picks: [0] }, { picks: [0, 2] }]);
  assert.deepEqual(answersFor(q, new Map([[0, new Set([1])]]), new Map([[1, " XL "]])), [{ picks: [1] }, { picks: [], other: "XL" }]);
});

test("approval card: Allow once sends approve --allow with the card's request, then the card goes", async () => {
  const cx = fakeCx(); const c = new Cards();
  await c.poll(cx, "t1", true);
  let el = c.view("t1", T);
  assert.match(textOf(el), /Bash command.*rm -rf build/);
  byId(el, "card-allow")._h.on_click({}, cx); await cx.settle();
  assert.deepEqual(acts[0].slice(0, 6), ["approve", "--terminal", "t1", "--request", "toolu_1", "--allow"]);
  assert.equal(c.view("t1", T), null);
});

test("Deny sends --deny; a changed card shows the new one with a note", async () => {
  const cx = fakeCx(); const c = new Cards();
  await c.poll(cx, "t1", true);
  fake.result = "changed"; const later = { ...approval, request: "toolu_2", command: "git push" };
  const exec0 = fake.exec; fake.exec = (cmd, args) => { const r = exec0(cmd, args); if (args[0] === "approve") cardNow = later; return r; };
  byId(c.view("t1", T), "card-deny")._h.on_click({}, cx); await cx.settle();
  assert.ok(acts[0].includes("--deny"));
  const el = c.view("t1", T);
  assert.match(textOf(el), /git push/);
  assert.match(textOf(byId(el, "card-note")), /changed on screen/);
});

test("question card: pick options, then Send answer sends the answers in order", async () => {
  cardNow = question;
  const cx = fakeCx(); const c = new Cards();
  await c.poll(cx, "t1", true);
  let el = c.view("t1", T);
  assert.match(textOf(el), /2 questions.*Pick a color\?.*Red.*Recommended/);
  assert.equal(byId(el, "card-answer")._h.on_click, undefined, "Send answer is off until each question has an answer");
  byId(el, "card-q0-o1")._h.on_click({}, cx);
  byId(el, "card-q1-o0")._h.on_click({}, cx);
  el = c.view("t1", T);
  byId(el, "card-q1-o2")._h.on_click({}, cx);
  el = c.view("t1", T);
  byId(el, "card-answer")._h.on_click({}, cx); await cx.settle();
  const i = acts[0].indexOf("--answers");
  assert.deepEqual(JSON.parse(acts[0][i + 1]), [{ picks: [1] }, { picks: [0, 2] }]);
  assert.ok(acts[0].includes("toolu_q"));
});

test("a question with unread ones says to answer in the terminal, without a send button", async () => {
  cardNow = { ...question, unread: 2 };
  const cx = fakeCx(); const c = new Cards(); await c.poll(cx, "t1", true);
  const el = c.view("t1", T);
  assert.match(textOf(el), /2 more questions follow/);
  assert.equal(byId(el, "card-answer"), null);
});

test("not wanted: the card is dropped and card is not read", async () => {
  const cx = fakeCx(); const c = new Cards(); await c.poll(cx, "t1", true);
  const before = log.exec.length;
  await c.poll(cx, "t1", false);
  assert.equal(c.card("t1"), null);
  assert.equal(log.exec.length, before);
});

test("Needs you shows the picked agent's card", async () => {
  const cx = fakeCx(); const v = new NeedsView(cx);
  let el = v.render(cx);
  assert.match(textOf(byId(el, "needs-detail")), /Reading what it waits for/);
  v.tick(cx); await cx.settle();
  el = v.render(cx);
  assert.ok(byId(el, "needs-card-allow"));
});

test("no view makes input state while it draws (herdr-app refuses it)", async () => {
  const { made } = await import("./stub/ui.mjs");
  const { ChatView } = await import("../mods/flockbook/chat.js");
  cardNow = { ...question };
  fake.exec = (_c, args) => args[0] === "card" ? jl(header("card"), cardNow, end)
    : args[0] === "feed" ? jl(header("feed"), { type: "log", agent: "claude", path: "/l", start: 0, size: 1, cursor: "c" }, { type: "splice", at: 0, count: 1 }, { type: "row", id: "u1", k: "user", text: "hi" }, end)
    : jl(header(args[0]), end);
  fake.agents = [agent("t1", "blocked"), agent("t2", "blocked")];
  const cx = fakeCx(); const chat = new ChatView(cx); const needs = new NeedsView(cx);
  await chat.tick(cx); needs.tick(cx); await cx.settle();
  const before = made.textarea;
  chat.render(cx); needs.render(cx);
  fake.shared["flockbook.agent"] = "t2"; await chat.tick(cx); await cx.settle();
  chat.render(cx); needs.render(cx);
  assert.equal(made.textarea, before);
});
