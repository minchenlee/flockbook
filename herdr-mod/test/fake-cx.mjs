// A fake cx: collects spawned tasks so a test can await them, counts notify calls.
export function fakeCx() {
  const cx = {
    tasks: [], notified: 0, timers: [],
    notify() { cx.notified++; },
    spawn(f) { const p = f(cx); cx.tasks.push(p); return p; },
    timer: { every(ms, f) { cx.timers.push({ ms, f }); return 0; }, after(ms, f) { cx.timers.push({ ms, f, once: true }); return 0; } },
    async settle() { while (cx.tasks.length) await cx.tasks.shift(); },
  };
  return cx;
}
