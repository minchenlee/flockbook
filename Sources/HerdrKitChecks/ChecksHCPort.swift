import Foundation
@testable import HerdrKit

// hc beside the herdr server: one backend's snapshot never stands for the other's. FakeHC only.

/// Pulled texts are pruned per backend: a TUI pane's only after a TUI snapshot without it, an hc pane's only after an
/// hc snapshot without it. hc answering first (the herdr server down or slower) must not delete the TUI's texts.
@MainActor func checksHCPrunePulled() async throws {
    let fake = FakeHC(path: fakeHCPath())
    try fake.start()
    defer { fake.kill() }
    fake.terminals = [FakeHC.terminal("t1")]
    let store = HerdrStore(session: HerdrSession(name: "x", socketPath: "/nonexistent"), hc: HCSession(socketPath: fake.path))
    let pulled = MemoryPulledStore()
    for pane in ["w1:p1", "hc:t1", "hc:t9"] { _ = pulled.keep(PulledText(pane: pane, device: PulledText.mac, text: "text of \(pane)")) }
    pulled.prune(gone: store.paneGone)
    check(pulled.all.count == 3, "hc prune: no snapshot yet: nothing pruned (\(pulled.all.map(\.pane)))")
    await store.refreshHC()
    pulled.prune(gone: store.paneGone)
    check(pulled.all.map(\.pane).sorted() == ["hc:t1", "w1:p1"], "hc prune: an hc snapshot prunes gone hc panes only (\(pulled.all.map(\.pane)))")
    store.took([])
    pulled.prune(gone: store.paneGone)
    check(pulled.all.map(\.pane) == ["hc:t1"], "hc prune: a TUI snapshot without the pane prunes it (\(pulled.all.map(\.pane)))")
}