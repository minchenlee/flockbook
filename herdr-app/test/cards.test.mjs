import { test, beforeEach } from "node:test";
import assert from "node:assert/strict";
import { fake, log, reset } from "./stub/herdr.mjs";
import { textOf, byId } from "./stub/ui.mjs";
import { fakeCx } from "./fake-cx.mjs";
import { agent, space } from "./fixtures.mjs";
import { Cards, wanted, answersFor, draftFor } from "../mods/flockbook/cards.js";
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

test("an agent Flockbook cannot tell apart: the card shows the sidecar's reason, not 'another agent'", async () => {
  const cx = fakeCx(); const c = new Cards();
  await c.poll(cx, "t1", true);
  fake.result = "no-identity"; fake.message = "Flockbook cannot tell this agent from another program in this terminal, so it sends nothing. Use the terminal.";
  byId(c.view("t1", T), "card-allow")._h.on_click({}, cx); await cx.settle();
  const note = textOf(byId(c.view("t1", T), "card-note"));
  assert.match(note, /cannot tell this agent/);
  assert.doesNotMatch(note, /Another agent/);
  fake.result = undefined; fake.message = undefined;
});

test("another press was being handled at the same moment: it says press again, and the buttons stay on", async () => {
  cardNow = approval;
  const cx = fakeCx(); const c = new Cards(); await c.poll(cx, "t1", true);
  const n = acts.length;
  fake.result = "busy"; fake.message = "Another press was being handled at the same moment, so this one sent nothing. Press again.";
  byId(c.view("t1", T), "card-allow")._h.on_click({}, cx); await cx.settle();
  const el = c.view("t1", T);
  assert.match(textOf(byId(el, "card-note")), /Press again/);
  assert.doesNotMatch(textOf(el), /Answer sent/, "not shown as answered");
  fake.result = undefined; fake.message = undefined;
  assert.notEqual(byId(el, "card-allow")._h.on_click, undefined, "Allow is still on");
  byId(el, "card-allow")._h.on_click({}, cx); await cx.settle();
  assert.equal(acts.length, n + 2, "the second press goes to the sidecar");
});

// Issue #11, item 4: the sidecar's `blocked` (the rule `answer` refuses with) shows from the start: Send off, the
// reason, and a button that opens the terminal.
const LABELS = "The terminal's review screen could not show which options were picked (an option with a comma or a ● or → mark, or two options alike). Answer in the terminal.";
const OWN = "Your own answer has a comma, a ● or → mark, or is the same as an option, so the terminal's review screen could not show it. Change it, or answer in the terminal.";

test("a question the review cannot show: Send off from the start, the reason, and Answer in terminal", async () => {
  cardNow = { ...question, blocked: LABELS };
  const cx = fakeCx(); const c = new Cards(cx); await c.poll(cx, "t1", true);
  let el = c.view("t1", T);
  assert.match(textOf(byId(el, "card-blocked")), /review screen could not show/, "said on the first question already");
  byId(el, "card-q0-o1")._h.on_click({}, cx);
  byId(c.view("t1", T), "card-q1-o0")._h.on_click({}, cx);
  el = c.view("t1", T);
  assert.equal(byId(el, "card-answer")._h.on_click, undefined, "Send is off");
  assert.equal(c.key_down({ key: "enter", modifiers: {} }, cx, "t1"), false, "Enter sends nothing");
  await cx.settle();
  assert.equal(acts.length, 0, "nothing went to the sidecar");
  byId(el, "card-terminal")._h.on_click({}, cx); await cx.settle();
  assert.equal(log.overlays.length, 1, "the terminal opens");
  assert.match(JSON.stringify(log.overlays[0]), /"terminal":"t1"/);
});

test("an own answer the review cannot show: checked again as it changes, Send off until it is fine", async () => {
  cardNow = question;
  const cards = [];
  fake.exec = (_c, args) => {
    if (args[0] !== "card") return jl(header(args[0]), end);
    cards.push(args);
    const i = args.indexOf("--answers");
    const a = i < 0 ? [] : JSON.parse(args[i + 1]);
    return jl(header("card"), a.some((x) => (x.other ?? "").includes(",")) ? { ...question, blocked: OWN } : question, end);
  };
  const cx = fakeCx(); const c = new Cards(cx); await c.poll(cx, "t1", true);
  assert.ok(!cards[0].includes("--answers"), "nothing typed yet: the labels only");
  byId(c.view("t1", T), "card-q0-o1")._h.on_click({}, cx);
  c.view("t1", T);
  const type = async (text) => { c.others[1].set_value(text); c.others[1]._h.change({}, cx); await c.poll(cx, "t1", true); };
  await type("x, y");
  const last = () => JSON.parse(cards.at(-1)[cards.at(-1).indexOf("--answers") + 1]);
  assert.deepEqual(last(), [{ picks: [1] }, { picks: [], other: "x, y" }], "a change reads the card again at once, with the answers so far");
  let el = c.view("t1", T);
  assert.match(textOf(byId(el, "card-blocked")), /Change it, or answer in the terminal/);
  assert.equal(byId(el, "card-answer")._h.on_click, undefined, "Send is off");
  await type("olive");
  el = c.view("t1", T);
  assert.equal(byId(el, "card-blocked"), null, "fine now");
  assert.notEqual(byId(el, "card-answer")._h.on_click, undefined, "Send is on");
  assert.equal(c.others[1].value(), "olive", "the typed text stays");
});

test("draftFor includes picks; pick() resets throttle e.at so poll updates blocked right away", async () => {
  const q = question.question;
  assert.equal(draftFor(q, new Map(), new Map()), null, "empty picks and others -> null");
  assert.deepEqual(
    draftFor(q, new Map([[0, new Set([1])]]), new Map()),
    [{ picks: [1] }, { picks: [] }],
    "draftFor includes picks and { picks: [] } for unanswered questions",
  );

  cardNow = { ...question, blocked: LABELS };
  const calls = [];
  fake.exec = (_c, args) => {
    if (args[0] !== "card") return jl(header(args[0]), end);
    calls.push(args);
    const i = args.indexOf("--answers");
    const a = i < 0 ? [] : JSON.parse(args[i + 1]);
    return jl(header("card"), a[0]?.picks?.includes(0) ? question : { ...question, blocked: LABELS }, end);
  };
  const cx = fakeCx(); const c = new Cards(cx);
  await c.poll(cx, "t1", true);
  assert.ok(!calls[0].includes("--answers"));
  byId(c.view("t1", T), "card-q0-o0")._h.on_click({}, cx);
  assert.equal(c.entry("t1").at, 0, "pick() resets e.at = 0 on a question card");
  await c.poll(cx, "t1", true); // non-forced poll runs immediately because e.at === 0
  assert.equal(calls.length, 2, "next poll reads card again with the new pick");
  const sentDraft = JSON.parse(calls[1][calls[1].indexOf("--answers") + 1]);
  assert.deepEqual(sentDraft, [{ picks: [0] }, { picks: [] }]);
  assert.equal(byId(c.view("t1", T), "card-blocked"), null, "picking option 0 clears blocked");
});

test("review-screen stop: when act fails and poll drops card to null, a read-only stop banner stays", async () => {
  cardNow = question;
  fake.exec = (_c, args) => {
    if (args[0] === "card") return jl(header("card"), cardNow ?? { type: "card" }, end);
    if (args[0] === "answer") {
      acts.push(args);
      cardNow = null; // Claude moved to Review screen where card is null
      return jl(header("answer"), { type: "done", result: "changed", message: "Review screen did not match; finish in the terminal." }, end);
    }
    return jl(header(args[0]), end);
  };
  const cx = fakeCx(); const c = new Cards(cx);
  await c.poll(cx, "t1", true);
  byId(c.view("t1", T), "card-q0-o0")._h.on_click({}, cx);
  byId(c.view("t1", T), "card-q1-o0")._h.on_click({}, cx);
  byId(c.view("t1", T), "card-answer")._h.on_click({}, cx);
  await cx.settle();

  const el = c.view("t1", T);
  assert.notEqual(el, null, "stop banner remains visible when card became null with an error/stop note");
  assert.match(textOf(byId(el, "card-note")), /Review screen did not match/);
  assert.equal(byId(el, "card-q0-o0"), null, "option picking is hidden on stop banner");
  assert.equal(byId(el, "card-answer"), null, "Send answer is hidden on stop banner");
  assert.equal(c.key_down({ key: "1", modifiers: {} }, cx, "t1"), false, "digit keys disabled");
  assert.equal(c.key_down({ key: "enter", modifiers: {} }, cx, "t1"), false, "Enter disabled");

  // Subsequent poll while still wanted retains the stop banner; opening terminal works
  await c.poll(cx, "t1", true, true);
  assert.notEqual(byId(c.view("t1", T), "card-note"), null);
  byId(c.view("t1", T), "card-terminal")._h.on_click({}, cx); await cx.settle();
  assert.equal(log.overlays.length, 1);

  // Once agent is no longer wanted, banner clears
  await c.poll(cx, "t1", false);
  assert.equal(c.view("t1", T), null);
});

test("Show all screen- card re-checks blocked on kept full question set without --all keys", async () => {
  const yesNoQ = {
    toolUseId: "screen-yesno",
    agent: "claude",
    delivery: "picker",
    state: "pending",
    questions: [
      {
        id: "Q1?",
        header: "One",
        text: "Q1?",
        multi: false,
        options: [
          { label: "Yes", detail: "", recommended: false },
          { label: "Yes", detail: "", recommended: false },
          { label: "No", detail: "", recommended: false },
        ],
      },
      {
        id: "Q2?",
        header: "Two",
        text: "Q2?",
        multi: false,
        options: [
          { label: "A", detail: "", recommended: false },
          { label: "B", detail: "", recommended: false },
        ],
      },
    ],
  };
  const firstScreen = {
    type: "card",
    kind: "question",
    request: "screen-yesno",
    unread: 1,
    question: { ...yesNoQ, questions: yesNoQ.questions.slice(0, 1) },
  };
  const fullScreen = {
    type: "card",
    kind: "question",
    request: "screen-yesno",
    unread: 0,
    question: yesNoQ,
    blocked: LABELS,
  };
  let alls = 0;
  const pollArgs = [];
  fake.exec = (_c, args) => {
    if (args[0] !== "card") return jl(header(args[0]), end);
    if (args.includes("--all")) {
      alls += 1;
      return jl(header("card"), fullScreen, end);
    }
    pollArgs.push(args);
    const qi = args.indexOf("--question");
    if (qi >= 0) {
      const passedQ = JSON.parse(args[qi + 1]);
      const ai = args.indexOf("--answers");
      const a = ai >= 0 ? JSON.parse(args[ai + 1]) : [];
      const blocked = a[0]?.picks?.includes(2) ? undefined : LABELS;
      const { blocked: _drop, ...baseFull } = fullScreen;
      return jl(header("card"), { ...baseFull, question: passedQ, ...(blocked ? { blocked } : {}) }, end);
    }
    return jl(header("card"), firstScreen, end);
  };

  const cx = fakeCx(); const c = new Cards(cx);
  await c.poll(cx, "t1", true);
  byId(c.view("t1", T), "card-all")._h.on_click({}, cx); await cx.settle();
  assert.equal(alls, 1);
  assert.notEqual(byId(c.view("t1", T), "card-blocked"), null, "initial colliding labels block Send");

  // Pick unique "No" (option 2) on Q1 and "A" (option 0) on Q2; non-forced poll updates blocked without --all.
  byId(c.view("t1", T), "card-q0-o2")._h.on_click({}, cx);
  byId(c.view("t1", T), "card-q1-o0")._h.on_click({}, cx);
  await c.poll(cx, "t1", true);
  assert.equal(alls, 1, "no extra --all key presses on poll");
  let el = c.view("t1", T);
  assert.equal(byId(el, "card-blocked"), null, "picking unique 'No' clears blocked on Show all card");
  assert.notEqual(byId(el, "card-answer")._h.on_click, undefined, "Send answer enables once blocked clears");

  // Go back to Q1 and switch to colliding "Yes" (option 0): re-blocks without --all.
  byId(el, "card-back")._h.on_click({}, cx);
  byId(c.view("t1", T), "card-q0-o0")._h.on_click({}, cx);
  await c.poll(cx, "t1", true);
  assert.equal(alls, 1, "still no --all key presses");
  el = c.view("t1", T);
  assert.notEqual(byId(el, "card-blocked"), null, "picking colliding 'Yes' restores blocked");
  assert.equal(byId(el, "card-answer")._h.on_click, undefined, "Send answer disables again");
});
