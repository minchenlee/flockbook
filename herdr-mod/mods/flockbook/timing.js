// Render and poll timing for performance checks. Off unless the file <app home>/flockbook/timing
// exists (only a test home has it). Once a second it writes one line per measured name to the
// app's log: how many times, the average and the slowest, in ms.
import { appHome, readTextFile } from "herdr";

let on = false;
const stats = new Map();
export const now = () => globalThis.performance?.now?.() ?? Date.now();
export const timingOn = () => on;

export async function loadTiming() {
  try { on = (await readTextFile(`${appHome()}/flockbook/timing`)) !== null; } catch { on = false; }
  return on;
}

// Adds one sample. `extra`: a number summed per second (rows drawn, bytes read).
export function record(name, ms, extra = 0) {
  if (!on) return;
  const s = stats.get(name) ?? { n: 0, sum: 0, max: 0, extra: 0 };
  s.n++; s.sum += ms; s.max = Math.max(s.max, ms); s.extra += extra;
  stats.set(name, s);
}

export function flush() {
  if (!on || !stats.size) return;
  const parts = [...stats].map(([k, s]) => `${k} n=${s.n} avg=${(s.sum / s.n).toFixed(2)} max=${s.max.toFixed(2)}${s.extra ? ` x=${s.extra}` : ""}`);
  console.log(`[flockbook-timing] ${new Date().toISOString().slice(11, 19)} ${parts.join(" | ")}`);
  stats.clear();
}
