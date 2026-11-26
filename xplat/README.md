# xplat

Cross-platform (Linux / Windows / macOS) spike for Flockbook in Rust + iced 0.14.
Phase A crates are spikes; each one answers one gate.

| Crate | Gate |
|---|---|
| `herdr-core` | Herdr link: the TUI socket client and the herdr-core (`hc`) client, snapshot, the conversation parsers (Claude Code, Codex, pi, Cursor, Grok, agy), the session resolver and log locator, status line records, question pickers, approval rules and the key gate; ported from Swift `Sources/HerdrKit` with its checks |
| `flockbook-modfeed` | The Herdr app mod's sidecar (`herdr-app/docs/protocol.md`): the only sidecar (the Swift one was retired) |
| `spike-select` | Drag-select and copy across rendered markdown blocks |
| `spike-virt` | Virtual list of 5,000+ messages, memory and frame-time bench |
| `spike-ime` | Text input with CJK preedit |

All spikes run headless (no window). Run `cargo test` in this directory.

## The mod's sidecar

`flockbook-modfeed` prints the protocol's JSON Lines (protocol 1). Linux and macOS: Unix socket.
Windows builds, but herdr-core's Windows endpoint and the open-files scan are TODOs: there it answers `no-core`
unless `HC_SOCKET` is set, and it fails closed.

```sh
cargo build -p flockbook-modfeed
FLOCKBOOK_MODFEED=$PWD/target/debug/flockbook-modfeed npm --prefix ../mod test   # the mod against this binary
```
