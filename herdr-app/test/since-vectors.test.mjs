// The "Since you left" card and Summaries: the cases every side shares (Tests/HerdrKitTests/Fixtures/since-vectors.json).
// The Mac app (Sources/HerdrKitChecks/ChecksSince.swift) and the helper (xplat/flockbook-modfeed/tests/since_vectors.rs)
// read the same file: a text or rule changed on one side only fails here or there.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { span, words, about, arrive, countLine, AWAY_MS } from "../mods/flockbook/since.js";
import { facts, SUMMARY_BATCH } from "../mods/flockbook/glance.js";
import { SETTINGS, usageText } from "../mods/flockbook/settings.js";

const v = JSON.parse(readFileSync(fileURLToPath(new URL("../../Tests/HerdrKitTests/Fixtures/since-vectors.json", import.meta.url)), "utf8"));

test("since vectors: the counts' words and the 5-minute mark", () => {
  assert.equal(AWAY_MS, v.away_ms);
  for (const [ms, want] of v.span) assert.equal(span(ms), want, String(ms));
  for (const [text, want] of v.words) assert.equal(words(text), want, text);
  for (const [n, want] of v.about) assert.equal(about(n), want, String(n));
  for (const [s, want] of v.count_line) assert.equal(countLine(s), want);
  for (const c of v.arrive) {
    const g = c.id ? { id: c.id } : {};
    const got = arrive("t1", c.seen, g, c.now);
    assert.deepEqual(got && { from: got.from, back: got.back }, c.want, c.why);
  }
});

test("since vectors: the line from the log", () => {
  for (const [g, want] of v.facts) assert.equal(facts(g), want, JSON.stringify(g));
  for (const l of v.logs) assert.equal(facts(l.want), l.line, l.name);
});

test("since vectors: the Summaries settings say the same", () => {
  const s = (k) => SETTINGS.find((x) => x.key === k);
  assert.equal(s("summaries").about, v.texts.summaries_about);
  assert.equal(s("summary_by").about, v.texts.by_about);
  assert.deepEqual(s("summary_by").notes, v.texts.by_notes);
  assert.equal(SUMMARY_BATCH, v.summary_batch);
  for (const [u, want] of v.usage_text) assert.equal(usageText(u), want);
  const src = readFileSync(fileURLToPath(new URL("../mods/flockbook/settings.js", import.meta.url)), "utf8");
  for (const text of [v.texts.summaries_sent, v.texts.model_about, ...Object.values(v.texts.model_empty), ...Object.values(v.texts.model_source)]) {
    assert.ok(src.includes(JSON.stringify(text)), text);
  }
  const card = readFileSync(fileURLToPath(new URL("../mods/flockbook/since.js", import.meta.url)), "utf8");
  for (const k of ["card_title", "summary_mark", "from_log", "from_log_with_summary", "jump"]) assert.ok(card.includes(`"${v.texts[k]}"`), k);
  assert.ok(card.includes("`Needs you: ${"), "needs_prefix");
  assert.equal(v.texts.needs_prefix, "Needs you: ");
});
