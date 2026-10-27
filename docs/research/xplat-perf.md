# Cross-platform preview (iced): performance

Measured 2026-10-27 on an Apple Silicon Mac, macOS 26, iced 0.14. Headless only: no window is opened.

## The bench

`xplat/flockbook/examples/perf.rs` drives the app the way the iced runtime does: `view`, `UserInterface::build` (layout),
`update` with the frame's events, `draw`. It runs on synthetic data (no herdr connection).

```sh
cd xplat
cargo run --release -p flockbook --example perf                  # all scenarios, tiny-skia, record only
cargo run --release -p flockbook --example perf idle scroll      # some scenarios
cargo run --release -p flockbook --example perf idle --wgpu --raster   # wgpu, read the frame back each time
```

- `--raster` renders every frame (`screenshot`). On wgpu that includes a GPU readback of about 5 ms CPU, so the
  `idle: empty window (baseline)` line is printed and subtracted.
- `--quick` runs a tenth of the frames (useful in a debug build).
- `FB_PERF_ROUNDS=n` sets the number of updates in the `updates` scenario.
- Scenarios: `startup`, `idle`, `scroll`, `earlier`, `updates`, `overview`, `terminal`.

Numbers vary by about 2× between runs when other work runs on the machine. Compare runs made one after the other.

What the bench does not measure: the present of a real window (swap chain, compositor). A real window pays it once per
redraw.

## Debug and release

Before, UI work in a debug build was about 10× slower (scroll layout ~5 ms instead of ~0.4 ms; one Tick ~3 ms of update
alone). tiny-skia rasterization in a debug build took 1.5–8 s a frame. Most of that time was in dependencies (iced,
wgpu, cosmic-text, tiny-skia), compiled without optimization.

Now `[profile.dev.package."*"]` builds dependencies at `opt-level = 2` (the app's own crates stay at 0). Debug build,
record only:

| | Before | After |
|---|---|---|
| Scroll 2000 items, frame p50 | ~5 ms | 0.8 ms |
| Tick | ~8 ms | 0.33 ms |
| Idle UI work, 6 working tabs | | 2.4 ms a second (0.2% of one core) |
| `refresh()` with 2000 items | 20 ms | 9.6 ms (the app's own code, unoptimized) |

Cost: the first dev build after this change compiles all dependencies again (7.5 min here), and `target/debug` grows by
the optimized copies (4.5 → 8.7 GB here, because the old copies stay until a clean).

## Fixes

| Fix | Before | After |
|---|---|---|
| A new conversation tail keeps the head's row heights (`Doc::set_rows`). Before, every new item re-estimated all rows. | `refresh()` with 2000 items: 9.7 ms (release), 20 ms (debug) | 2.4 ms (release); next frame 0.16 ms |
| Load earlier (+1600 items onto 400), same fix | 26.8 ms (release), 77 ms (debug) | 19.9 ms (release) |
| Working arcs ask the runtime for their next redraw. Before, a 60 ms `Spin` timer sent a message, so each frame ran `update`, `view` and layout. | arc frame: update + view + layout + draw, 0.06–0.08 ms | draw only, 0.03 ms |
| In the background (window not focused: another app in front, or minimized), working arcs ask for a redraw every 100 ms. | 16.7 redraws a second | 10 a second; arc CPU 0.9 → 0.6 ms a second (record only) |
| Status tags are kept per pane for a second. Before, every view read and parsed the status line file. | one file read per message | at most one per second per pane |

Idle with 6 working tabs (demo), release, wgpu with readback, CPU over the empty-window baseline:

| | UI work per second | of one core |
|---|---|---|
| Before | 11.5 ms | 1.2% |
| After | 7.3 ms | 0.7% |

Record only (tiny-skia, no raster): 0.7–2.1 ms per second (0.1–0.2% of one core), 18.5 wakeups per second
(arc redraws 16.7, Tick 1.25, composer queue poll 0.6).

## Measurements after the fixes (release, tiny-skia, record only)

| Scenario | p50 | p95 |
|---|---|---|
| Startup: fonts + renderer | 85–335 ms | |
| Startup: first frame of a 2000-item conversation | 3.6 ms | |
| Scroll 2000 items, new rows | 0.34 ms | 1.0 ms |
| Scroll 2000 items, seen rows | 0.72 ms | 2.6 ms |
| New item while pinned to the bottom (frame) | 0.35 ms | 0.78 ms |
| Overview with 200 tabs, scroll frame | 1.3 ms | 1.9 ms |
| Overview with 200 tabs, arc redraw | 0.57 ms | 0.99 ms |
| Terminal, 6 KB of output per poll | 0.57 ms | 1.2 ms |
| Memory, 2000 items after scrolling | 42 MB | |

## Not done

- The 800 ms Tick still rebuilds the view when nothing changed (about 0.1–0.3 ms each in release, 1.25 a second). The
  cards' screen poll, the other split panes, the age labels and the status tags all update on it, so gating it needs a
  change counter in the store. Not worth the risk for this cost.
- A window that is focused but covered gets no event from iced 0.14 (winit's `Occluded` is not forwarded), so its arcs
  keep the front rate. Whether macOS skips the present of a covered window cannot be measured headless.
