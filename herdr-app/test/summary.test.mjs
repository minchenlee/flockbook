// The setting Summaries (off by default): two lines by a model on the Since you left card, above the line from the log.
import { test, beforeEach } from "node:test";
import assert from "node:assert/strict";
import { fake, log, reset } from "./stub/herdr.mjs";
import { textOf, byId } from "./stub/ui.mjs";
import { fakeCx } from "./fake-cx.mjs";
import { agent, space } from "./fixtures.mjs";
import { forgetCommands } from "../mods/flockbook/sidecar.js";
import { KEY_CONFIG, setting } from "../mods/flockbook/config.js";
import { SETTINGS, SettingsView, usageText } from "../mods/flockbook/settings.js";
import { KEY_GLANCE, KEY_SUMMARY, SUMMARY_BATCH, forgetGlance, pollGlance, summarize, summaryFor } from "../mods/flockbook/glance.js";
import { sinceCard } from "../mods/flockbook/since.js";
import { theme } from "./stub/herdr.mjs";

const now = Date.now();
const gline = (terminal, extra = {}) => ({ type: "glance", terminal, agent: "claude", path: "/l.jsonl", id: `${terminal}-last`, edited: 2, commands: 1,
  tests: "passed", test: "pytest", said: "The parser skips empty lines.", ended: now - 60000, ...extra });
const on = (yes = true) => { fake.shared[KEY_CONFIG] = { file: yes ? { summaries: "on" } : {}, bad: false, text: "{}" }; };
// A helper with `glance` and `summary`. `answers`: terminal → the helper's line or error for it.
const helper = (lines, answers, commands = ["version", "glance", "summary"]) => (_cmd, argv) => {
  const which = argv[0];
  const body = which === "version" ? [{ type: "version", commands }] : which === "glance" ? lines
    : which === "summary" ? [answers[argv[2]] ?? { type: "error", code: "summary-failed", message: "no" }]
    : which === "summary-usage" ? (answers.usage ?? [])
    : which === "summary-models" ? [answers.models?.[argv[2]] ?? { type: "error", code: "models-failed", message: "no" }] : [];
  return { code: 0, stdout: [{ type: "header", protocol: 1, sidecar: "0.1.0", cmd: which }, ...body, { type: "end", ok: true, ms: 1 }].map((o) => JSON.stringify(o)).join("\n") };
};
const asked = () => log.exec.filter((e) => e[1][0] === "summary").map((e) => e[1][2]);
const sum = (terminal, state, needs = null) => ({ type: "summary", terminal, id: `${terminal}-last`, state, needs, model: "claude-haiku-5-5", cached: false });

beforeEach(() => {
  reset(); forgetGlance(); forgetCommands();
  fake.spaces = [space("sp1", { name: "shop", tabs: [{ id: "a", status: "done", name: "importer" }, { id: "b", status: "done", name: "docs" }] })];
  fake.agents = [agent("t1", "done", { tab: "a", tab_name: "importer" }), agent("t2", "done", { tab: "b", tab_name: "docs" })];
});

test("summaries: off by default; the setting says what is sent and where", () => {
  assert.equal(setting("summaries"), "off");
  const s = SETTINGS.find((x) => x.key === "summaries");
  assert.deepEqual(s.choices.map((c) => c[0]), ["off", "on"]);
  assert.match(s.about, /A model writes two lines about an agent's last turn, for the chat's Since you left card\. Off: nothing is sent to a model\./);
  const by = SETTINGS.find((x) => x.key === "summary_by");
  assert.equal(setting("summary_by"), "own");
  assert.match(by.notes.own, /A turn goes only where that conversation already goes/);
  assert.match(by.notes.codex, /The turns of other agents go to Codex's provider too/);
  const v = new SettingsView(fakeCx());
  const el = v.render(fakeCx());
  assert.match(textOf(byId(el, "set-summaries")), /Summaries/);
  assert.equal(byId(el, "set-summaries-off")._a.bg, "#selected");
  // Off: the rows for the program and the model are not there.
  assert.equal(byId(el, "set-summary_by"), null);
  assert.equal(byId(el, "set-summary_model"), null);
  on();
  const shown = v.render(fakeCx());
  assert.match(textOf(byId(shown, "set-summaries")), /the end of its last turn and your message that started it go to the model\. A model can be wrong; the line from the log stays on the card\./);
  assert.match(textOf(byId(shown, "set-summary_by")), /Written by .*Each agent's own Claude Code Codex pi .*Other agents get no summary\./);
  assert.equal(byId(shown, "set-summary_model"), null, "each agent's own program runs its default model");
});

test("summaries off: the helper is never asked, and no card changes", async () => {
  on(false);
  fake.exec = helper([gline("t1"), gline("t2")], { t1: sum("t1", "x") });
  const cx = fakeCx();
  await pollGlance(cx, true);
  assert.deepEqual(asked(), []);
  assert.equal(fake.shared[KEY_SUMMARY], undefined);
  assert.equal(summaryFor("t1"), null);
  // A summary kept from before it was turned off is not shown.
  fake.shared[KEY_SUMMARY] = { t1: { id: "t1-last", by: "own/", state: "Old words.", needs: null } };
  assert.equal(summaryFor("t1"), null);
});

test("summaries on: each stopped turn is asked once, a few agents at a time; a failure keeps the facts", async () => {
  on();
  fake.agents.push(agent("t3", "working"), agent("t4", "done"), agent("t5", "done"), agent("t6", "blocked"));
  const lines = [gline("t1"), gline("t2"), gline("t3"), gline("t4", { agent: "agy" }), gline("t5", { ended: now - 13 * 3600000 }), gline("t6")];
  fake.exec = helper(lines, { t1: sum("t1", "The importer is fixed and tested.", "Say whether to update the changelog.") });
  const cx = fakeCx();
  await pollGlance(cx, true);
  assert.deepEqual(asked(), ["t1", "t2", "t5"], "the agents that stopped, the newest first, in one batch: one agent a call; one that stopped 13 hours ago too");
  assert.deepEqual(log.exec.find((e) => e[1][0] === "summary")[1].slice(0, 5), ["summary", "--terminal", "t1", "--cache", "/home/test/.config/herdr-app/flockbook/cache.json"]);
  assert.deepEqual(summaryFor("t1"), { id: "t1-last", by: "own/", v: 5, state: "The importer is fixed and tested.", needs: "Say whether to update the changelog.", model: "claude-haiku-5-5" });
  assert.deepEqual(fake.shared[KEY_SUMMARY].t2, { id: "t2-last", by: "own/", v: 5, failed: true });
  assert.equal(summaryFor("t2"), null, "a failed summary shows nothing");
  await pollGlance(cx, true); await pollGlance(cx, true);
  assert.deepEqual(asked(), ["t1", "t2", "t5"], "not again for the same turns; none for a working or waiting agent, or one whose program writes none");
  // The agent wrote more: the old words are for another turn, and the new turn is asked.
  fake.shared[KEY_GLANCE] = { on: true, by: { ...fake.shared[KEY_GLANCE].by, t1: gline("t1", { id: "t1-next" }) } };
  assert.equal(summaryFor("t1"), null);
  await summarize(cx);
  assert.deepEqual(asked(), ["t1", "t2", "t5", "t1"]);
  // A summary kept by an older mod (it asked for other lines): it shows, and its turn is asked once more.
  fake.shared[KEY_GLANCE] = { on: true, by: { ...fake.shared[KEY_GLANCE].by, t1: gline("t1") } };
  fake.shared[KEY_SUMMARY] = { ...fake.shared[KEY_SUMMARY], t1: { id: "t1-last", by: "own/", state: "Old words.", needs: null } };
  assert.equal(summaryFor("t1").state, "Old words.");
  await summarize(cx); await summarize(cx);
  assert.deepEqual(asked(), ["t1", "t2", "t5", "t1", "t1"]);
  assert.equal(summaryFor("t1").v, 5);
});

test("summaries on, many agents stopped: at most three are asked at a time, the rest the next time", async () => {
  on();
  assert.equal(SUMMARY_BATCH, 3);
  fake.agents = ["t1", "t2", "t3", "t4", "t5"].map((t) => agent(t, "done"));
  const answers = Object.fromEntries(["t1", "t2", "t3", "t4"].map((t) => [t, sum(t, `About ${t}.`)]));
  fake.exec = helper(fake.agents.map((a) => gline(a.address.replace(/^hc:/, ""))), answers);
  const cx = fakeCx();
  await pollGlance(cx, true);
  assert.deepEqual(asked(), ["t1", "t2", "t3"]);
  assert.deepEqual(["t1", "t2", "t3", "t4"].map((t) => summaryFor(t)?.state ?? null), ["About t1.", "About t2.", "About t3.", null]);
  await summarize(cx);
  assert.deepEqual(asked(), ["t1", "t2", "t3", "t4", "t5"]);
  assert.equal(summaryFor("t4").state, "About t4.");
  assert.deepEqual(fake.shared[KEY_SUMMARY].t5, { id: "t5-last", by: "own/", v: 5, failed: true }, "one that fails does not stop the others");
  await summarize(cx);
  assert.equal(asked().length, 5);
});

test("summaries on, two copies of the mod look at the same time: one asks, the other sees it busy", async () => {
  fake.exec = helper([gline("t1"), gline("t2")], { t1: sum("t1", "About t1."), t2: sum("t2", "About t2.") });
  const cx = fakeCx();
  await pollGlance(cx, true);
  on(); forgetCommands();
  await Promise.all([summarize(cx), summarize(cx)]);
  assert.deepEqual(asked(), ["t1", "t2"], "each turn once");
  await summarize(cx);
  assert.deepEqual(asked(), ["t1", "t2"], "free again after: nothing new to ask");
});

test("summaries on, an older helper without `summary`: nothing is asked", async () => {
  on();
  fake.exec = helper([gline("t1")], {}, ["version", "glance"]);
  await pollGlance(fakeCx(), true);
  assert.deepEqual(asked(), []);
});

test("summaries on, but off in the helper's file: the turn is not marked as failed", async () => {
  on();
  fake.exec = helper([gline("t1")], { t1: { type: "error", code: "summaries-off", message: "off" } });
  await pollGlance(fakeCx(), true);
  assert.equal(fake.shared[KEY_SUMMARY]?.t1, undefined);
});

test("the Since you left card names a summary as a model's words, and keeps the line from the log", () => {
  on();
  fake.agents = [agent("t1", "done", { tab_name: "importer" })];
  fake.shared[KEY_GLANCE] = { on: true, by: { t1: gline("t1") } };
  fake.shared[KEY_SUMMARY] = { t1: { id: "t1-last", by: "own/", state: "The importer is fixed.", needs: "Answer the changelog question." } };
  const s = { away: "47 minutes", messages: 2, words: 40, facts: "Edited 2 files", said: null };
  const card = textOf(sinceCard(theme(), s, summaryFor("t1"), () => {}, () => {}));
  assert.match(card, /Summary · written by a model, may be wrong The importer is fixed\. Needs you: Answer the changelog question\. From the log, not from the summary: Edited 2 files/);
  assert.match(textOf(sinceCard(theme(), s, null, () => {}, () => {})), /words From the log: Edited 2 files/);
});

test("summary written by: each agent's own program by default; one program reads every agent, with the model named", async () => {
  on(); forgetGlance();
  fake.agents = [agent("t1", "done"), agent("t2", "done"), agent("t3", "done")];
  const lines = [gline("t1"), gline("t2", { agent: "codex" }), gline("t3", { agent: "agy" })];
  fake.exec = helper(lines, { t1: sum("t1", "One."), t2: sum("t2", "Two."), t3: sum("t3", "Three.") });
  const cx = fakeCx();
  await pollGlance(cx, true);
  for (let i = 0; i < 4; i++) await summarize(cx);
  assert.deepEqual(asked(), ["t1", "t2"], "own: Claude Code, Codex and pi agents; no program can write for the fourth");
  // One program for all: every agent's turn is asked again, by it.
  fake.shared[KEY_CONFIG] = { file: { summaries: "on", summary_by: "pi", summary_model: " opencode-go/glm-5.3-flash " }, bad: false, text: "{}" };
  assert.equal(setting("summary_model"), "opencode-go/glm-5.3-flash");
  assert.equal(summaryFor("t1"), null, "the words of the program before are not shown as this one's");
  for (let i = 0; i < 4; i++) await summarize(cx);
  assert.deepEqual(asked(), ["t1", "t2", "t1", "t2", "t3"]);
  assert.equal(summaryFor("t3").by, "pi/opencode-go/glm-5.3-flash");
});

test("settings: the chosen program's models are listed under the field; a click or a typed name sets the model", async () => {
  const models = { type: "models", by: "pi", source: "program", models: Array.from({ length: 11 }, (_, i) => ({ id: `go/m${i}`, note: "1M context" })).concat([{ id: "go/glm-flash", note: "262K context" }]) };
  fake.exec = helper([], { models: { "pi": models } }, ["version", "glance", "summary", "summary-models"]);
  let file = JSON.stringify({ summaries: "on", summary_by: "pi" });
  fake.readText = (p) => (p.endsWith("/flockbook/config.json") ? file : null);
  fake.writeText = (p, text) => { if (p.endsWith("/flockbook/config.json")) file = text; return null; };
  const cx = fakeCx();
  const v = new SettingsView(cx);
  await cx.settle();
  let el = v.render(fakeCx());
  assert.equal(byId(el, "set-summary_by-pi")._a.bg, "#selected");
  assert.match(textOf(byId(el, "set-summary_by")), /pi writes for every agent/);
  const row = () => textOf(byId(el, "set-summary_model"));
  assert.match(row(), /Default the model pi uses by default go\/m0 1M context/);
  assert.match(row(), /4 more\. Type a part of a name to see fewer\. From pi --list-models\./);
  assert.equal(byId(el, "set-model-default")._a.bg, "#selected");
  // Typing narrows the list.
  v.model.set_value("glm");
  el = v.render(fakeCx());
  assert.match(row(), /go\/glm-flash 262K context/);
  assert.doesNotMatch(row(), /go\/m0/);
  // A click sets it.
  const c2 = fakeCx();
  byId(el, "set-model-0")._h.on_click({}, c2);
  await c2.settle();
  assert.equal(setting("summary_model"), "go/glm-flash");
  assert.equal(v.model.value(), "go/glm-flash");
  el = v.render(fakeCx());
  assert.equal(byId(el, "set-model-default")._a.bg, undefined);
  assert.match(row(), /go\/m0/, "the saved name in the field: the whole list again");
  // Another program: the model of the one before is dropped, and that program's list is asked.
  const c3 = fakeCx();
  byId(el, "set-summary_by-codex")._h.on_click({}, c3);
  await c3.settle();
  assert.deepEqual([setting("summary_by"), setting("summary_model"), v.model.value()], ["codex", "", ""]);
  el = v.render(fakeCx());
  assert.match(row(), /Empty: the model Codex uses by default\. The list of models could not be read\./);
  // Each agent's own: no model row.
  const c4 = fakeCx();
  byId(el, "set-summary_by-own")._h.on_click({}, c4);
  await c4.settle();
  assert.equal(byId(v.render(fakeCx()), "set-summary_model"), null);
});

test("settings: with Summaries on, a line says how often a model ran and how much text went", async () => {
  const use = (since, runs, failed, sent, got) => ({ type: "summary-usage", since, runs, failed, sent, got, ms: 1000 * runs });
  assert.equal(usageText(null), "");
  assert.equal(usageText({ today: use(2, 0, 0, 0, 0), week: use(1, 0, 0, 0, 0) }), "No model ran for a summary in the last 7 days.");
  fake.exec = helper([], { usage: [use(2, 42, 1, 96321, 5149), use(1, 210, 0, 512000, 1)] }, ["version", "glance", "summary", "summary-usage"]);
  let file = JSON.stringify({ summaries: "on" });
  fake.readText = (p) => (p.endsWith("/flockbook/config.json") ? file : null);
  const cx = fakeCx();
  const v = new SettingsView(cx);
  await cx.settle();
  assert.match(textOf(byId(v.render(fakeCx()), "set-summaries")), /stays on the card\. Today: 42 runs of a model \(1 failed\), about 96,000 characters sent and 5,100 back\. Last 7 days: 210 runs of a model, about 512,000 characters sent and 1 back\./);
  const asked = log.exec.find((e) => e[1][0] === "summary-usage")[1];
  assert.deepEqual([asked[1], asked[3], asked[5], Number(asked[4]) - Number(asked[6])], ["--cache", "--since", "--since", 6 * 86400000], "from this morning, and from 6 days before it");
  // An older helper: no line, and the command is not run.
  reset(); forgetCommands();
  fake.exec = helper([], {}, ["version", "glance", "summary"]);
  fake.readText = (p) => (p.endsWith("/flockbook/config.json") ? file : null);
  const c2 = fakeCx();
  const v2 = new SettingsView(c2);
  await c2.settle();
  assert.match(textOf(byId(v2.render(fakeCx()), "set-summaries")), /stays on the card\. Off On$/);
  assert.equal(log.exec.some((e) => e[1][0] === "summary-usage"), false);
});
