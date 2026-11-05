import { test, beforeEach } from "node:test";
import assert from "node:assert/strict";
import { fake, log, reset } from "./stub/herdr.mjs";
import { textOf, byId, calls, lastList } from "./stub/ui.mjs";
import { fakeCx } from "./fake-cx.mjs";
import { agent, space } from "./fixtures.mjs";
import { ChatView, chipIcon, oneLine } from "../mods/flockbook/chat.js";
import { parseAnswer } from "../mods/flockbook/sidecar.js";
import { emptyFeed, applyFeed, applyOlder, olderFrom } from "../mods/flockbook/feedstate.js";
import { wrapLines, markdownHeight, rowHeight, LH } from "../mods/flockbook/estimate.js";

const jl = (...lines) => ({ code: 0, stdout: lines.map((l) => JSON.stringify(l)).join("\n") + "\n", stderr: "" });
const header = { type: "header", protocol: 1, sidecar: "0.1.0", command: "feed" };
const end = { type: "end", ok: true };
const logLine = (cursor, path = "/logs/s.jsonl") => ({ type: "log", agent: "claude", path, start: 0, size: 10, cursor, model: "claude-opus-5-5" });
const user = (id, text) => ({ type: "row", id, k: "user", text, ts: 1 });
const asst = (id, text) => ({ type: "row", id, k: "assistant", text, ts: 2 });
const tools = (id, done = true) => ({ type: "row", id, k: "tools", ts: 3, chips: [{ symbol: "terminal", text: "Ran 2 commands",
  calls: [{ name: "Bash", summary: "swift test", done, error: false }, { name: "Bash", summary: "git status", done: true, error: false }] }] });

beforeEach(() => {
  reset(); calls.length = 0;
  fake.spaces = [space()];
  fake.agents = [agent("t1", "working")];
});

test("parseAnswer splits header, end, error and the lines between", () => {
  const a = parseAnswer(jl(header, logLine("c1"), { type: "error", code: "read", message: "x" }, end).stdout + "not json\n");
  assert.equal(a.header.protocol, 1); assert.equal(a.end.ok, true); assert.equal(a.error.code, "read");
  assert.deepEqual(a.lines.map((l) => l.type), ["log"]);
});

test("applyFeed: splice replaces from `at`, unchanged keeps rows, a new log path starts over", () => {
  const f = emptyFeed("t1");
  assert.equal(applyFeed(f, parseAnswer(jl(header, logLine("c1"), { type: "splice", at: 0, count: 2 }, user("u1", "hi"), asst("a1", "yo"), end).stdout)), 0);
  assert.deepEqual(f.rows.map((r) => r.id), ["u1", "a1"]); assert.equal(f.cursor, "c1");
  assert.equal(applyFeed(f, parseAnswer(jl(header, logLine("c1"), { type: "unchanged" }, end).stdout)), -1);
  assert.equal(applyFeed(f, parseAnswer(jl(header, logLine("c2"), { type: "splice", at: 1, count: 3 }, asst("a1", "yo!"), tools("a1-t2"), end).stdout)), 1);
  assert.deepEqual(f.rows.map((r) => r.text ?? r.k), ["hi", "yo!", "tools"]);
  applyFeed(f, parseAnswer(jl(header, logLine("c3", "/logs/other.jsonl"), { type: "splice", at: 3, count: 4 }, user("u9", "new"), end).stdout));
  assert.deepEqual(f.rows.map((r) => r.id), ["u9"], "rows of the old log are dropped");
});

test("applyFeed: an error shows once, and the next good answer clears it", () => {
  const f = emptyFeed("t1");
  const bad = { header: null, end: null, lines: [], error: { code: "core", message: "down" } };
  assert.equal(applyFeed(f, bad), 0);
  assert.equal(applyFeed(f, bad), -1);
  assert.equal(applyFeed(f, parseAnswer(jl(header, logLine("c1"), { type: "unchanged" }, end).stdout)), 0);
  assert.equal(f.error, null);
});

test("estimates: wide characters wrap sooner; code and headings add height", () => {
  assert.ok(wrapLines("中".repeat(100), 400) > wrapLines("a".repeat(100), 400));
  assert.ok(markdownHeight("# Title\n\ntext", 600) > markdownHeight("text", 600));
  assert.ok(markdownHeight("```\na\nb\n```", 600) > markdownHeight("a b", 600));
  const t = tools("x").chips ? { k: "tools", chips: tools("x").chips } : null;
  assert.ok(rowHeight(t, 600, true) > rowHeight(t, 600, false));
  assert.ok(rowHeight({ k: "assistant", text: "line\n\n".repeat(400) }, 600) > 4000);
});

test("estimates: code boxes and tables are guessed at their drawn height (no wrap margin, no gap)", () => {
  // codeBox: 6 px margin + 10 px padding + 1 px border, each side; 19 px per line.
  assert.equal(markdownHeight("```\na\nb\nc\n```", 600), 3 * 19 + 34);
  // table: 6 px margin + 1 px border each side; a row is 8 + 21 + 8 px plus a 1 px line (1 px high on the head row, on purpose).
  assert.equal(markdownHeight("| A | B |\n|---|---|\n| a | b |", 600), 14 + 17 + 21 + 17 + 21);
  // The prose gets a small pad at the end, once per text, so a long answer with code is not guessed far too tall.
  const prose = "word ".repeat(30);
  const code = "```\n" + "x = 1\n".repeat(30) + "```";
  const pad = markdownHeight("a", 600) - LH;
  // The blank lines around a box add nothing: the box's own margin is the gap.
  const both = markdownHeight(`${prose}\n\n${code}\n\n${prose}`, 600);
  assert.ok(Math.abs(both - (2 * markdownHeight(prose, 600) - pad + markdownHeight(code, 600))) <= 1, `${both}`);  // 1 px: rounding
  // Between two paragraphs the blank line stays a 10 px gap.
  assert.equal(markdownHeight("a\n\nb", 600), 2 * LH + 10 + pad);
});

test("chipIcon maps SF Symbols names to the mod's icons", () => {
  assert.match(chipIcon("terminal"), /terminal\.svg$/);
  assert.match(chipIcon("doc.text"), /doc\.svg$/);
  assert.match(chipIcon("magnifyingglass"), /search\.svg$/);
  assert.match(chipIcon("hammer"), /tool\.svg$/);
});

test("the chat loads the shown agent's feed, opens at the newest row, and passes the cursor back", async () => {
  fake.exec = (cmd, args) => {
    if (!args.includes("--cursor")) return jl(header, logLine("c1"), { type: "splice", at: 0, count: 3 }, user("u1", "Fix the login bug"), asst("a1", "Looking at it."), tools("a1-t2", false), end);
    return jl(header, logLine("c2"), { type: "splice", at: 2, count: 4 }, tools("a1-t2"), { type: "row", id: "f-a1", k: "footer", text: "01:41 · Worked 36s · 2 tools" }, end);
  };
  const cx = fakeCx();
  const v = new ChatView(cx); await cx.settle();
  const feeds = () => log.exec.filter(([, a]) => a[0] === "feed");
  const [cmd, args] = feeds()[0];
  assert.match(cmd, /flockbook\/bin\/flockbook-modfeed$/);
  assert.deepEqual(args.slice(0, 3), ["feed", "--terminal", "t1"]);
  v.width = 900;
  v.render(cx);
  assert.equal(lastList._args.n, 3);
  assert.deepEqual(calls.at(-1), ["scroll_to_item", 2, "top"]);
  const rendered = lastList._args.render({ start: 0, end: 3 });
  assert.match(textOf(rendered[0]), /Fix the login bug/);
  assert.match(textOf(rendered[2]), /Ran 2 commands/);

  v.lastPoll = 0; await v.tick(cx); await cx.settle();
  assert.ok(feeds()[1][1].includes("c1"), "the second feed passes the cursor");
  v.render(cx);
  assert.equal(lastList._args.n, 4);
  assert.match(textOf(lastList._args.render({ start: 3, end: 4 })[0]), /Worked 36s/);
});

test("a click on a tool row shows its calls; the row grows", async () => {
  fake.exec = () => jl(header, logLine("c1"), { type: "splice", at: 0, count: 1 }, tools("a1-t2"), end);
  const cx = fakeCx(); const v = new ChatView(cx); await cx.settle(); v.width = 900;
  v.render(cx); const closed = lastList._args.sizes[0];
  lastList._h.on_item_click("a1-t2", cx);
  v.render(cx);
  assert.ok(lastList._args.sizes[0] > closed);
  assert.match(textOf(lastList._args.render({ start: 0, end: 1 })[0]), /git status/);
});

test("a tool call with a multi-line input shows one line, cut with …, and its row is clipped", async () => {
  const many = { type: "row", id: "a1-t3", k: "tools", ts: 3, chips: [{ symbol: "terminal", text: "Ran 3 commands\nmore",
    calls: [
      { name: "Bash", summary: "cd /x; python3 - <<'EOF'\np = 'a'\nprint(p)\nEOF", done: true, error: false },
      { name: "Bash", summary: "\r\n  \r\necho one\r\necho two", done: true, error: false },
      { name: "Bash", summary: "git status", done: true, error: false },
    ] }] };
  fake.exec = () => jl(header, logLine("c1"), { type: "splice", at: 0, count: 1 }, many, end);
  const cx = fakeCx(); const v = new ChatView(cx); await cx.settle(); v.width = 900;
  v.render(cx);
  lastList._h.on_item_click("a1-t3", cx);
  v.render(cx);
  const el = lastList._args.render({ start: 0, end: 1 })[0];
  const text = textOf(el);
  assert.ok(!/[\r\n]/.test(text), JSON.stringify(text));
  assert.match(text, /cd \/x; python3 - <<'EOF' …/);
  assert.match(text, /echo one …/);
  assert.match(text, /git status(?! …)/);
  assert.match(text, /Ran 3 commands …/);
  // Each call row is one line tall and clips what does not fit.
  const rows = [];
  const walk = (n) => { if (!n || typeof n !== "object") return; if (n._a?.h === 26) rows.push(n); (n.kids || []).forEach(walk); };
  walk(el);
  assert.equal(rows.length, 3);
  for (const r of rows) assert.ok("overflow_hidden" in r._a, "a call row clips its content");
  assert.equal(v.height(lastList._args.render ? v.feed().rows[0] : null), rowHeight(many, v.colWidth(), true));
  assert.equal(rowHeight(many, 760, true), 40 + 3 * 26 + 8);
});

test("oneLine keeps the first line with text and marks the rest with …", () => {
  assert.equal(oneLine("a\nb"), "a …");
  assert.equal(oneLine("\n\n  x  \r\n"), "x");
  assert.equal(oneLine("one"), "one");
  assert.equal(oneLine(""), "");
  assert.equal(oneLine(undefined), "");
  assert.equal(oneLine("tab\there"), "tab here");
});

test("scrolling up stops following and fades in the ↓ button; reaching the end follows again", async () => {
  fake.exec = () => jl(header, logLine("c1"), { type: "splice", at: 0, count: 2 }, user("u1", "a"), asst("a1", "b"), end);
  const cx = fakeCx(); const v = new ChatView(cx); await cx.settle(); v.width = 900;
  let el = v.render(cx);
  assert.equal(byId(el, "jump")._a.opacity, 0, "hidden while following");
  const find = (n) => (n?._h?.on_scroll_wheel ? n : (n?.kids ?? []).map(find).find(Boolean));
  find(el)._h.on_scroll_wheel({ delta: { x: 0, y: 40 } }, cx);
  el = v.render(cx);
  assert.equal(byId(el, "jump")._a.opacity, 1);
  assert.ok(byId(el, "jump").kids.some((k) => k.tag === "svg" && k._a.path.endsWith("arrow-down.svg")));
  byId(el, "jump")._h.on_click({}, cx);
  assert.equal(v.follow, true);
  assert.deepEqual(calls.at(-1), ["scroll_to_item", 1, "top"]);
});

test("a tall row shows in full: its whole guessed height, no Show all", async () => {
  const long = "paragraph\n\n".repeat(300);
  fake.exec = () => jl(header, logLine("c1"), { type: "splice", at: 0, count: 1 }, asst("a1", long), end);
  const cx = fakeCx(); const v = new ChatView(cx); await cx.settle(); v.width = 900;
  v.render(cx);
  assert.ok(lastList._args.sizes[0] > 4000);
  assert.doesNotMatch(textOf(lastList._args.render({ start: 0, end: 1 })[0]), /Show all/);
});

test("messages for a missing sidecar, a wrong protocol and a non-agent terminal", async () => {
  const show = async (exec) => { reset(); fake.spaces = [space()]; fake.agents = [agent("t1", "idle")]; fake.exec = exec;
    const cx = fakeCx(); const v = new ChatView(cx); await cx.settle(); return textOf(v.render(cx)); };
  const missing = await show(null);
  assert.match(missing, /sidecar did not answer/);
  assert.match(missing, /helper is missing\. Get it from github\.com\/minchenlee\/flockbook\/releases\./);
  assert.match(await show(() => jl({ ...header, protocol: 2 }, end)), /protocol 2; this mod needs 1/);
  assert.match(await show(() => jl(header, { type: "error", code: "not-agent", message: "t1 runs no agent" }, { type: "end", ok: false })), /runs no agent/);
  assert.match(await show(() => jl(header, logLine("c1", null), { type: "splice", at: 0, count: 0 }, end)), /No conversation yet/);
});

test("switching agents shows the other agent's rows", async () => {
  fake.agents = [agent("t1", "idle"), agent("t2", "idle")];
  fake.exec = (_c, args) => jl(header, logLine("c-" + args[2]), { type: "splice", at: 0, count: 1 }, user("u-" + args[2], "from " + args[2]), end);
  const cx = fakeCx(); const v = new ChatView(cx); await cx.settle(); v.width = 900;
  fake.shared["flockbook.agent"] = "t2";
  await v.tick(cx); await cx.settle();
  v.render(cx);
  assert.match(textOf(lastList._args.render({ start: 0, end: 1 })[0]), /from t2/);
});

test("a call without a result shows as running, not as an approval", async () => {
  fake.exec = () => jl(header, logLine("c1"), { type: "splice", at: 0, count: 1 },
    { type: "row", id: "p1", k: "perm", text: "Bash command", perm: { title: "Bash command", command: "swift test", tool: "Bash", toolUseId: "toolu_1", state: "pending" } }, end);
  const cx = fakeCx(); const v = new ChatView(cx); await cx.settle(); v.width = 900;
  v.render(cx);
  const text = textOf(lastList._args.render({ start: 0, end: 1 })[0]);
  assert.match(text, /Running · Bash command/);
  assert.doesNotMatch(text, /approval/i);
});

test("cmd-alt-up / down switch spaces from the chat; other keys pass", async () => {
  const { spaceKey } = await import("../mods/flockbook/chat.js");
  const m = (x) => ({ shift: false, control: false, alt: false, platform: false, function: false, ...x });
  assert.equal(spaceKey({ key: "up", modifiers: m({ platform: true, alt: true }) }), "space.prev");
  assert.equal(spaceKey({ key: "down", modifiers: m({ platform: true, alt: true }) }), "space.next");
  assert.equal(spaceKey({ key: "up", modifiers: m({ platform: true }) }), null);
  assert.equal(spaceKey({ key: "a", modifiers: m({ platform: true, alt: true }) }), null);
});

test("every chat opens at its newest row: switching back to a read chat, and coming back from another screen", async () => {
  fake.agents = [agent("t1", "idle"), agent("t2", "idle")];
  fake.exec = (_c, args) => jl(header, logLine("c-" + args[2]), { type: "splice", at: 0, count: 2 }, user("u-" + args[2], "a"), asst("a-" + args[2], "b"), end);
  const cx = fakeCx(); const v = new ChatView(cx); await cx.settle(); v.width = 900;
  fake.shared["flockbook.agent"] = "t2"; await v.tick(cx); await cx.settle();
  v.follow = false;                                  // scrolled up in t2
  fake.exec = () => jl(header, logLine("c-t1"), { type: "unchanged" }, end);
  calls.length = 0;
  fake.shared["flockbook.agent"] = "t1"; await v.tick(cx); await cx.settle();
  assert.equal(v.follow, true);
  assert.deepEqual(calls.find((c) => c[0] === "scroll_to_item"), ["scroll_to_item", 1, "top"], "t1 is read and unchanged, still at its end");
  fake.shared["flockbook.screen"] = "overview"; await v.tick(cx);
  v.follow = false; calls.length = 0;
  fake.shared["flockbook.screen"] = "chat"; await v.tick(cx);
  assert.equal(v.follow, true);
  assert.deepEqual(calls.find((c) => c[0] === "scroll_to_item"), ["scroll_to_item", 1, "top"]);
});

test("applyOlder puts older rows on top; a new tail start drops them; start 0 means the first row is shown", () => {
  const f = emptyFeed("t1");
  const tail = (start, ...rows) => parseAnswer(jl(header, { ...logLine("c1"), start }, { type: "splice", at: 0, count: rows.length }, ...rows, end).stdout);
  applyFeed(f, tail(5000, user("u3", "c"), asst("a3", "d")));
  assert.equal(olderFrom(f), 5000);
  const older = (start, ...rows) => parseAnswer(jl(header, { type: "older", path: "/logs/s.jsonl", start, count: rows.length }, ...rows, end).stdout);
  assert.equal(applyOlder(f, older(2000, user("u2", "b"), asst("a2", "x"), user("u3", "dup")), 5000), 2, "a row already shown is not added twice");
  assert.deepEqual(f.rows.map((r) => r.id), ["u2", "a2", "u3", "a3"]);
  assert.equal(olderFrom(f), 2000);
  assert.equal(applyOlder(f, older(0, user("u1", "a")), 5000), 0, "an answer for an old request is dropped");
  assert.equal(applyOlder(f, older(0, user("u1", "a")), 2000), 1);
  assert.equal(olderFrom(f), null);
  // A splice counts in tail rows; the index it returns counts what is shown.
  assert.equal(applyFeed(f, parseAnswer(jl(header, { ...logLine("c2"), start: 5000 }, { type: "splice", at: 1, count: 2 }, asst("a3", "d!"), end).stdout)), 4);
  assert.deepEqual(f.rows.map((r) => r.id), ["u1", "u2", "a2", "u3", "a3"]);
  applyFeed(f, tail(9000, user("u9", "z")));
  assert.deepEqual(f.rows.map((r) => r.id), ["u9"]); assert.equal(olderFrom(f), 9000);
});

test("reaching the top asks for older rows once and keeps the same row in view", async () => {
  fake.exec = (_c, args) => args.includes("--before")
    ? jl(header, { type: "older", path: "/logs/s.jsonl", start: 0, count: 2 }, user("u0", "old"), asst("a0", "older"), end)
    : jl(header, { ...logLine("c1"), start: 4000 }, { type: "splice", at: 0, count: 2 }, user("u1", "a"), asst("a1", "b"), end);
  const cx = fakeCx(); const v = new ChatView(cx); await cx.settle(); v.width = 900;
  await v.tick(cx); await cx.settle();   // the chat screen is up (its first tick opens it at the end)
  v.autoOlder = 5;   // loads by themselves used up (they fill a short chat): here only the wheel loads
  let el = v.render(cx); lastList._args.render({ start: 0, end: 1 });
  assert.ok(!v.wantOlder, "the first row on screen alone loads nothing");
  const wheel = (n) => (n?._h?.on_scroll_wheel ? n : (n?.kids ?? []).map(wheel).find(Boolean));
  wheel(el)._h.on_scroll_wheel({ delta: { x: 0, y: -40 } }, cx);
  assert.ok(!v.wantOlder, "scrolling down loads nothing");
  wheel(el)._h.on_scroll_wheel({ delta: { x: 0, y: 40 } }, cx);
  assert.equal(v.wantOlder, true);
  calls.length = 0;
  await v.tick(cx); await cx.settle();
  assert.deepEqual(v.feed().rows.map((r) => r.id), ["u0", "a0", "u1", "a1"]);
  assert.deepEqual(calls.filter((c) => c[0] === "scroll_to_item").at(-1), ["scroll_to_item", 2, "top"]);
  const asks = log.exec.filter((e) => e[1]?.includes?.("--before") || JSON.stringify(e).includes("--before"));
  assert.equal(asks.length, 1);
  assert.equal(olderFrom(v.feed()), null);
});

// A log whose last 2 MB holds few rows (big tool results): rows before `start`, read 2 MB at a time.
const shortTail = (olderAnswers) => {
  const asked = [];
  fake.exec = (_c, args) => {
    const i = args.indexOf("--before");
    if (i < 0) return jl(header, { ...logLine("c1"), start: 9000 }, { type: "splice", at: 0, count: 1 }, asst("a9", "last"), end);
    const before = Number(args[i + 1]); asked.push(before);
    const [start, rows] = olderAnswers(before);
    return jl(header, { type: "older", path: "/logs/s.jsonl", start, count: rows.length }, ...rows, end);
  };
  return asked;
};
const runTicks = async (v, cx, n = 12) => { for (let k = 0; k < n; k++) { v.render(cx); lastList._args.render({ start: 0, end: lastList._args.n }); await v.tick(cx); await cx.settle(); } };

test("rows that all fit on screen load older rows by themselves, until the log's start", async () => {
  const asked = shortTail((before) => [before - 3000, [user(`u${before}`, "q"), asst(`a${before}`, "r")]]);
  const cx = fakeCx(); const v = new ChatView(cx); await cx.settle(); v.width = 900;
  await runTicks(v, cx);
  assert.deepEqual(asked, [9000, 6000, 3000]);
  assert.equal(v.feed().rows.length, 7);
  assert.equal(olderFrom(v.feed()), null, "the start of the log was reached");
});

test("loading older rows by themselves stops after 5 loads per open (a log of huge tool results)", async () => {
  const asked = shortTail((before) => [before - 1, []]);   // only tool output: no rows, every time
  const cx = fakeCx(); const v = new ChatView(cx); await cx.settle(); v.width = 900;
  await runTicks(v, cx, 20);
  assert.deepEqual(asked, [9000, 8999, 8998, 8997, 8996], "a range with no rows goes on back, at most 5 times");
});

test("the first row on screen is the top, also when the last row is on screen too", async () => {
  shortTail(() => [0, []]);
  const cx = fakeCx(); const v = new ChatView(cx); await cx.settle(); v.width = 900;
  v.render(cx); lastList._args.render({ start: 0, end: 1 });
  assert.equal(v.topSeen, true);
});

test("older rows added while following the end keep the end in view and the ↓ button hidden", async () => {
  shortTail((before) => [0, [user("u0", "q"), asst("a0", "r")]]);
  const cx = fakeCx(); const v = new ChatView(cx); await cx.settle(); v.width = 900;
  calls.length = 0;
  await runTicks(v, cx, 3);
  assert.equal(v.feed().rows.length, 3);
  assert.equal(v.follow, true);
  assert.deepEqual(calls.filter((c) => c[0] === "scroll_to_item").at(-1), ["scroll_to_item", 2, "top"], "the newest row, not the old first one");
  assert.equal(byId(v.render(cx), "jump")._a.opacity, 0);
});

test("no element above a chat reply's text stops the press (Herdr 0.1.13: that makes the text unselectable)", async () => {
  fake.exec = () => jl(header, logLine("c1"), { type: "splice", at: 0, count: 2 }, user("u1", "hi"), asst("a1", "Some **text** to select."), end);
  const cx = fakeCx(); const v = new ChatView(cx); await cx.settle(); v.width = 900;
  const root = v.render(cx);
  const stops = [];
  const walk = (el, above) => {
    if (!el || typeof el !== "object") return;
    if (el.tag === "md") {
      for (const a of above) {
        const down = a._h?.on_mouse_down;
        if (!down) continue;
        let stopped = false;
        down({ button: "left", position: { x: 0, y: 0 } }, { stop_propagation: () => { stopped = true; }, notify() {} });
        if (stopped) stops.push(a.tag + (a._a?.id ? `#${a._a.id}` : ""));
      }
      return;
    }
    const kids = el.tag === "vl" ? el._args.render({ start: 0, end: el._args.n }) : (el.kids || []);
    for (const k of kids) walk(k, [...above, el]);
  };
  walk(root, []);
  assert.ok(lastList, "the chat drew its list");
  assert.deepEqual(stops, [], "an element above the reply text stops the left press");
});
