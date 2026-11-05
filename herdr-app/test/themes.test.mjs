// The dark `flockbook` theme keeps its own black-and-white accent and state colors on the same base colors
// as `flockbook-mac`, and its text stays readable on every base color.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const load = (name) => JSON.parse(readFileSync(new URL(`../themes/${name}.json`, import.meta.url), "utf8"));
const mono = load("flockbook"), mac = load("flockbook-mac");
const BASE = ["canvas", "chrome", "raised", "hover", "selected", "border"];

// WCAG contrast ratio of two #rrggbb colors.
const lum = (hex) => {
  const [r, g, b] = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16) / 255)
    .map((c) => (c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4));
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
};
const contrast = (a, b) => { const [x, y] = [lum(a), lum(b)].sort((p, q) => q - p); return (x + 0.05) / (y + 0.05); };

test("the flockbook dark theme has the flockbook-mac base colors", () => {
  assert.equal(mono.background, mac.background);
  for (const k of BASE) assert.equal(mono.colors[k], mac.colors[k], k);
});

test("the flockbook dark theme keeps its own black-and-white accent and state colors", () => {
  for (const k of ["accent", "done", "blocked"]) assert.equal(mono.colors[k], mono.foreground, k);
  assert.equal(mono.colors.working, mono.colors.muted);
});

test("text and muted text are readable on every base color; dim text on the canvas and chrome", () => {
  for (const k of ["canvas", "chrome", "raised", "hover"]) {
    assert.ok(contrast(mono.foreground, mono.colors[k]) >= 4.5, `foreground on ${k}`);
    assert.ok(contrast(mono.colors.muted, mono.colors[k]) >= 4.5, `muted on ${k}`);
  }
  // Dim is for times and hints: at least what it had on the old black canvas (3.29).
  for (const k of ["canvas", "chrome"]) assert.ok(contrast(mono.colors.dim, mono.colors[k]) >= 3.2, `dim on ${k}`);
  assert.ok(contrast(mono.foreground, mono.selection) >= 4.5, "text on the selection");
});
