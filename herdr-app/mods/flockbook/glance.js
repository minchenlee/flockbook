// What each agent's last turn did, for the chat's "Since you left" card (since.js). The facts come from the
// helper's `glance` (docs/protocol.md): fixed rules on the conversation log, no model and no network. A helper
// without `glance` (an older one): nothing here is on, and the chat looks as it did before.
import { shared, setShared, appHome, readTextFile, writeTextFile, agents } from "herdr";
import * as sidecar from "./sidecar.js";
import { setting, loadConfig, KEY_CONFIG } from "./config.js";

export const KEY_GLANCE = "flockbook.glance";      // { on, by: { terminal: glance line } }
// What the owner saw of a turn: { seen: { terminal: { id, at, since } } }.
// `seen.id`: the turn's newest item when its chat was last on screen; `at`: when; `since`: when it was
// on screen before that id came (the "Since you left" card counts from there).
export const KEY_MARKS = "flockbook.glance.marks";
// With the setting Summaries on: { terminal: { id, state, needs } } by a model, or { id, failed: true }.
export const KEY_SUMMARY = "flockbook.summary";
const KEY_SUMMARY_BUSY = "flockbook.summary.busy";  // when a copy of this mod started to ask (ms)
const SUMMARY_BUSY_MS = 90000;
const SUMMARY_V = 5;                                // what a summary holds (5: the state and what it needs only): an older one is asked again
export const SUMMARY_BATCH = 3;                     // the agents asked about at a time
const KEY_AT = "flockbook.glance.at";              // when a copy of this mod last asked (ms): the copies take turns
const KEY_DONE = "flockbook.glance.done";          // when the newest answer came (ms)
export const GLANCE_MS = 5000;
export const marksPath = () => `${appHome()}/flockbook/glance-marks.json`;

// "hc:t7" and "t7" are one terminal.
export const term = (address) => String(address ?? "").replace(/^hc:/, "");
export const glanceOn = () => shared(KEY_GLANCE)?.on === true;
export const glanceFor = (address) => (address ? shared(KEY_GLANCE)?.by?.[term(address)] ?? null : null);
const marks = () => shared(KEY_MARKS) ?? { seen: {} };
// When the newest `glance` answer came (ms), 0 before the first. The chat waits for one newer than its arrival.
export const glancedAt = () => shared(KEY_DONE) ?? 0;

let busy = false, last = 0, marksText, loaded = false;
// For tests: forget what was read.
export const forgetGlance = () => { busy = false; last = 0; marksText = undefined; loaded = false; };

async function loadMarks() {
  if (loaded) return;
  loaded = true;
  let text = null;
  try { text = await readTextFile(marksPath()); } catch { text = null; }
  marksText = text;
  if (shared(KEY_MARKS)) return;
  let v = null;
  try { v = text ? JSON.parse(text) : null; } catch { v = null; }
  setShared(KEY_MARKS, { seen: v?.seen && typeof v.seen === "object" ? v.seen : {} });
}

// Kept in a file, so a restart does not forget what was seen. A failed write only
// loses that: the marks stay for this run.
async function saveMarks(m) {
  setShared(KEY_MARKS, m);
  const text = JSON.stringify(m) + "\n";
  try { await writeTextFile(marksPath(), text, marksText ?? null); marksText = text; }
  catch {
    try { marksText = await readTextFile(marksPath()); await writeTextFile(marksPath(), text, marksText ?? null); marksText = text; } catch { /* next change tries again */ }
  }
}

// One `glance` for every agent, at most each GLANCE_MS (`force`: now). Redraws only when a line changed.
export async function pollGlance(cx, force = false) {
  if (busy || (!force && Date.now() - Math.max(last, shared(KEY_AT) ?? 0) < GLANCE_MS)) return;
  busy = true; last = Date.now(); setShared(KEY_AT, last);
  try {
    await loadMarks();
    if (!(await sidecar.hasCommand("glance"))) { if (shared(KEY_GLANCE)?.on) { setShared(KEY_GLANCE, { on: false, by: {} }); cx?.notify?.(); } return; }
    const r = await sidecar.glance();
    if (r.error) return;
    const by = Object.fromEntries(r.lines.filter((l) => l.type === "glance" && l.terminal).map((l) => [l.terminal, l]));
    setShared(KEY_DONE, Date.now());
    const was = shared(KEY_GLANCE);
    if (!was?.on || JSON.stringify(was.by) !== JSON.stringify(by)) { setShared(KEY_GLANCE, { on: true, by }); cx?.notify?.(); }
  } finally { busy = false; }
  await summarize(cx);
}

export const summariesOn = () => setting("summaries") === "on";
// The programs that can write a summary (the helper's summary.rs), and the person's choice as one word:
// a summary by another program or model is not shown, and its turn is asked again.
const WRITERS = ["claude", "codex", "pi"];
const writer = () => `${setting("summary_by")}/${setting("summary_by") === "own" ? "" : setting("summary_model")}`;
// The model's two lines for the turn this agent's card shows now; null when off, not there yet, or failed.
export function summaryFor(address) {
  if (!summariesOn() || !address) return null;
  const s = shared(KEY_SUMMARY)?.[term(address)], g = glanceFor(address);
  return s?.state && g?.id && s.id === g.id && s.by === writer() ? s : null;
}

// With Summaries on: asks the helper for the summaries of at most SUMMARY_BATCH agents at a time (one agent a
// call, the calls side by side), and one such batch at a time over every copy of this mod. Every agent that
// stopped and wrote something, however long ago, the newest first, and each turn once: a failed turn is not asked
// again (the card keeps the line from the log). With "Each agent's own" (summary_by: own): only the agents whose program
// can write one (Claude Code, Codex, pi).
export async function summarize(cx, now = Date.now()) {
  if (!glanceOn()) return;
  if (!shared(KEY_CONFIG)) await loadConfig();
  if (!summariesOn() || now - (shared(KEY_SUMMARY_BUSY) ?? 0) < SUMMARY_BUSY_MS) return;
  const have = shared(KEY_SUMMARY) ?? {}, by = writer(), own = setting("summary_by") === "own";
  const next = agents().filter((a) => {
    const g = glanceFor(a.address), had = have[term(a.address)];
    return a.status !== "working" && a.status !== "blocked" && g?.id && g.said && (!own || WRITERS.includes(g.agent))
      && !(had?.id === g.id && had.by === by && had.v === SUMMARY_V);
  }).sort((a, b) => (glanceFor(b.address).ended ?? 0) - (glanceFor(a.address).ended ?? 0)).slice(0, SUMMARY_BATCH);
  if (!next.length || !(await sidecar.hasCommand("summary"))) return;
  setShared(KEY_SUMMARY_BUSY, now);
  try {
    await Promise.all(next.map(async (a) => {
      const t = term(a.address), id = glanceFor(a.address).id;
      const r = await sidecar.summary(t);
      const s = r.error ? null : r.lines.find((l) => l.type === "summary" && l.state);
      // Off in the helper's file (changed by hand): do not mark the turn, it is asked when the setting is on there.
      if (r.error?.code === "summaries-off") return;
      // Each answer shows when it comes: a slow one does not hold the others back.
      setShared(KEY_SUMMARY, { ...(shared(KEY_SUMMARY) ?? {}), [t]: s ? { id: s.id, by, v: SUMMARY_V, state: s.state, needs: s.needs ?? null, model: s.model ?? null } : { id, by, v: SUMMARY_V, failed: true } });
      cx?.notify?.();
    }));
  } finally { setShared(KEY_SUMMARY_BUSY, 0); }
}

// The chat shows this agent's newest item now. `now`: ms.
export function markSeen(address, now = Date.now()) {
  const g = glanceFor(address);
  if (!g?.id) return;
  const m = marks(), t = term(address), was = m.seen?.[t];
  if (was?.id === g.id) {
    // Still the same turn on screen: only the time moves, and not on every tick.
    if (now - was.at < 60000) return;
    return saveMarks({ ...m, seen: { ...m.seen, [t]: { ...was, at: now } } });
  }
  return saveMarks({ ...m, seen: { ...m.seen, [t]: { id: g.id, at: now, since: was?.at ?? null } } });
}
export const seenFor = (address) => marks().seen?.[term(address)] ?? null;

const count = (n, one, many) => `${n} ${n === 1 ? one : many}`;
// The line from the log: "Edited 5 files · pytest passed after the last edit". "" when there is nothing.
export function facts(g) {
  if (!g) return "";
  const out = [];
  if (g.edited) out.push(`edited ${count(g.edited, "file", "files")}`);
  const after = g.edited ? " after the last edit" : "";
  const test = g.test || "a test command";
  if (g.tests === "passed") out.push(`${test} passed${after}`);
  else if (g.tests === "failed") out.push(`${test} failed${after}`);
  else if (g.tests === "ran") out.push(`${test} ran${after}, result not known`);
  else if (g.tests === "none") out.push("no test run after the last edit");
  if (g.failed && g.tests !== "failed") out.push(`the last command failed: ${g.failed}`);
  if (!out.length && g.commands) out.push(`ran ${count(g.commands, "command", "commands")}`);
  const s = out.join(" · ");
  return s ? s[0].toUpperCase() + s.slice(1) : "";
}
