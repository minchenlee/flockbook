// Fades an element in (and lifts it a little) each time its key changes: one frame at opacity 0
// without a transition, so it starts hidden, then the target 1 with a native transition.
import { div } from "gpui-kit";
import { FADE, SPRING } from "./state.js";

export class Appear {
  constructor() { this.ready = new Map(); this.pending = new Set(); }

  // `id`: the wrapper's stable identity; `key`: what it shows (a new key fades in again).
  wrap(child, id, key, cx, lift = 0) {
    const ready = this.ready.get(id) === key;
    if (!ready && !this.pending.has(id)) {
      this.pending.add(id);
      const done = (cx2) => { this.pending.delete(id); this.ready.set(id, key); cx2?.notify?.(); };
      if (cx?.timer?.after) cx.timer.after(16, done); else done(null);
    }
    const el = div().id(id).w_full().relative().child(child);
    if (!ready) return el.opacity(0).top(lift);
    return el.opacity(1).top(0).transition("opacity", FADE).spring("top", SPRING);
  }
}
