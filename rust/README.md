# Rust sidecar (preview)

`flockbook-modfeed` in Rust: a drop-in for the Swift sidecar of the Herdr app mod (`herdr-mod/`), so the mod can
run where Swift and Foundation do not. Same commands, same JSON Lines, protocol 1 (`herdr-mod/docs/protocol.md`).

- `herdrkit/`: the parts of `Sources/HerdrKit` the sidecar reaches (herdr-core client, log parsers, session
  resolver, status line records, question pickers, approval rules, key gate). No UI, no TUI backend.
- `flockbook-modfeed/`: `Sources/ModFeedKit` and the binary.

```sh
cd rust
cargo build            # target/debug/flockbook-modfeed
cargo test             # Rust ports of the HerdrKitChecks the sidecar touches, plus a fake herdr-core
FLOCKBOOK_MODFEED=$PWD/target/debug/flockbook-modfeed npm --prefix ../herdr-mod test   # the mod against this binary
```

Platforms: Linux and macOS (Unix socket). Windows builds, but herdr-core's Windows endpoint, process listing and
clipboard image are TODOs: there the sidecar answers `no-core` unless `HC_SOCKET` is set, and it fails closed.
