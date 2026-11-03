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

test("question card: one question at a time; a one-choice pick goes on; Back; Send answer on the last", async () => {
  cardNow = question;
  const cx = fakeCx(); const c = new Cards();
  await c.poll(cx, "t1", true);
  let el = c.view("t1", T);
  assert.match(textOf(el), /question 1 of 2.*Pick a color\?.*Red.*Recommended/);
  assert.doesNotMatch(textOf(el), /Which sizes/);
  assert.equal(byId(el, "card-next")._h.on_click, undefined, "Next is off until the question has an answer");
  byId(el, "card-q0-o1")._h.on_click({}, cx);
  el = c.view("t1", T);
  assert.match(textOf(el), /question 2 of 2.*Which sizes\?/);
  byId(el, "card-back")._h.on_click({}, cx); await cx.settle();
  assert.match(textOf(c.view("t1", T)), /Pick a color/);
  byId(c.view("t1", T), "card-dot-1")._h.on_click({}, cx);
  el = c.view("t1", T);
  assert.equal(byId(el, "card-answer")._h.on_click, undefined, "Send is off until each question has an answer");
  byId(el, "card-q1-o0")._h.on_click({}, cx);
  byId(c.view("t1", T), "card-q1-o2")._h.on_click({}, cx);
  byId(c.view("t1", T), "card-answer")._h.on_click({}, cx); await cx.settle();
  const i = acts[0].indexOf("--answers");
  assert.deepEqual(JSON.parse(acts[0][i + 1]), [{ picks: [1] }, { picks: [0, 2] }]);
  assert.ok(acts[0].includes("toolu_q"));
});

test("a question with unread ones: Show all reads every question (card --all), then Send answer shows", async () => {
  const first = { ...question, unread: 1, question: { ...question.question, questions: question.question.questions.slice(0, 1) } };
  cardNow = first;
  const cx = fakeCx(); const c = new Cards(); await c.poll(cx, "t1", true);
  let el = c.view("t1", T);
  assert.match(textOf(el), /Show all 2 questions.*1 more question follows/);
  assert.equal(byId(el, "card-answer"), null);
  fake.exec = ((prev) => (cmd, args) => args.includes("--all") ? (acts.push(args), jl(header("card"), question, end)) : prev(cmd, args))(fake.exec);
  byId(el, "card-all")._h.on_click({}, cx); await cx.settle();
  assert.ok(acts.some((a) => a.includes("--all")));
  await c.poll(cx, "t1", true, true);   // the screen still shows one question; the full card stays
  el = c.view("t1", T);
  assert.match(textOf(el), /question 1 of 2/);
  assert.ok(byId(el, "card-next"));
});

test("keys: 1-9 pick in the first open question, Enter sends; approval 1 allows, 2 denies", async () => {
  cardNow = question;
  const cx = fakeCx(); const c = new Cards(); await c.poll(cx, "t1", true);
  const key = (k, mods = {}) => c.key_down({ key: k, modifiers: mods }, cx, "t1");
  assert.equal(key("enter"), false, "nothing picked yet");
  assert.equal(key("2"), true, "picks Blue and goes on to question 2");
  assert.equal(key("1", { platform: true }), false, "cmd-1 is the tab key");
  assert.equal(key("7"), false, "no 7th option");
  key("1"); key("3");
  assert.equal(key("enter"), true); await cx.settle();
  const i = acts[0].indexOf("--answers");
  assert.deepEqual(JSON.parse(acts[0][i + 1]), [{ picks: [1] }, { picks: [0, 2] }]);
  cardNow = approval; await c.poll(cx, "t1", true, true);
  assert.equal(key("2"), true); await cx.settle();
  assert.ok(acts.at(-1).includes("--deny"));
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

test("a question with unread ones never reads the others without a click", async () => {
  cardNow = { ...question, unread: 1, question: { ...question.question, questions: question.question.questions.slice(0, 1) } };
  let alls = 0;
  fake.exec = ((prev) => (cmd, args) => args.includes("--all") ? (alls++, jl(header("card"), question, end)) : prev(cmd, args))(fake.exec);
  const cx = fakeCx(); const c = new Cards();
  await c.poll(cx, "t1", true); await cx.settle();
  await c.poll(cx, "t1", true, true); await cx.settle();
  assert.equal(alls, 0, "no keys on a timer");
  assert.ok(byId(c.view("t1", T), "card-all"));
});

test("the card has no outer border; Next and Send sit at the right end, after the hint", async () => {
  cardNow = question;
  const cx = fakeCx(); const c = new Cards(); await c.poll(cx, "t1", true);
  const el = c.view("t1", T);
  assert.equal(el._a.border, undefined);
  const row = (function find(n) { return n?.kids?.some?.((k) => k._a?.id === "card-next") ? n : (n?.kids ?? []).map(find).find(Boolean); })(el);
  const ids = row.kids.map((k) => k._a?.id ?? (textOf(k).includes("picks") ? "hint" : "gap"));
  assert.equal(ids.at(-1), "card-next");
  assert.ok(ids.indexOf("hint") < ids.indexOf("card-next"));
});

test("two views of one card: Show all in one, both show every question", async () => {
  cardNow = { ...question, unread: 1, question: { ...question.question, questions: question.question.questions.slice(0, 1) } };
  let alls = 0;
  fake.exec = ((prev) => (cmd, args) => args.includes("--all") ? (alls++, jl(header("card"), question, end)) : prev(cmd, args))(fake.exec);
  const cx = fakeCx(); const a = new Cards(), b = new Cards();
  await a.poll(cx, "t1", true); await b.poll(cx, "t1", true);
  byId(a.view("t1", T), "card-all")._h.on_click({}, cx); await cx.settle();
  await b.poll(cx, "t1", true, true); await cx.settle();
  assert.equal(alls, 1);
  assert.match(textOf(b.view("t1", T)), /question 1 of 2/);
});

test("own answer: a row like the options; clicks and keys in it stay in the text box", async () => {
  cardNow = question;
  const cx = fakeCx(); const c = new Cards(cx); await c.poll(cx, "t1", true);
  const row = byId(c.view("t1", T), "card-q0-other");
  assert.match(textOf(row), /3/, "numbered after the options");
  let stopped = 0; const ecx = { ...cx, stop_propagation() { stopped++; } };
  row._h.on_mouse_down?.({}, ecx);
  row._h.on_key_down({ key: "1", modifiers: {} }, ecx);
  assert.equal(stopped, 2, "the card neither takes the focus nor picks option 1");
  assert.equal(c.byAgent.get("t1").picks.size, 0);
});

test("the number after the options moves the keyboard into the own-answer box", async () => {
  cardNow = question;
  const cx = fakeCx(); const c = new Cards(cx); await c.poll(cx, "t1", true);
  let focused = 0; c.others[0].focus = () => { focused++; };
  assert.equal(c.key_down({ key: "3", modifiers: {} }, cx, "t1"), true);
  assert.equal(focused, 1);
});

test("another agent in the pane: the card says so, nothing sent", async () => {
  cardNow = approval;
  fake.result = "changed"; fake.message = "another agent is in this pane now; nothing was sent";
  const cx = fakeCx(); const c = new Cards(); await c.poll(cx, "t1", true);
  byId(c.view("t1", T), "card-allow")._h.on_click({}, cx); await cx.settle();
  assert.match(textOf(byId(c.view("t1", T), "card-note")), /Another agent is in this pane now/);
});

test("an approval already answered and still on screen: buttons off, 1 and 2 do nothing, it says so", async () => {
  const cx = fakeCx(); const c = new Cards();
  // The key went out but the agent has not redrawn yet: the card comes back marked answered.
  fake.result = "unconfirmed"; fake.message = "no answer in time; the key may still arrive";
  await c.poll(cx, "t1", true);
  const exec0 = fake.exec; fake.exec = (cmd, args) => { const r = exec0(cmd, args); if (args[0] === "approve") cardNow = { ...approval, answered: true }; return r; };
  byId(c.view("t1", T), "card-allow")._h.on_click({}, cx); await cx.settle();
  assert.equal(acts.length, 1);
  const el = c.view("t1", T);
  assert.match(textOf(el), /may still arrive/);
  assert.equal(byId(el, "card-allow")._h.on_click, undefined, "Allow is off");
  assert.equal(byId(el, "card-deny")._h.on_click, undefined, "Deny is off");
  assert.match(textOf(el), /Answer sent/);
  assert.equal(c.key_down({ key: "1", keystroke: { key: "1" } }, cx, "t1"), false);
  await cx.settle();
  assert.equal(acts.length, 1, "no second approve");
  fake.message = undefined; fake.result = undefined;
});
