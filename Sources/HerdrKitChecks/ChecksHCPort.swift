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

/// A composer's attachments go to the TUI's remote host only for TUI panes; an hc pane's go through `terminal.put`.
@MainActor func checksHCComposerRemote() {
    let host = RemoteHost(via: .local)
    let tui = Pane(id: "w1:p1", terminalId: "p1", cwd: "/tmp", agent: nil)
    let hc = Pane(id: "hc:t1", terminalId: "t1", cwd: "/tmp", agent: nil, backend: .hc)
    check(tui.composerRemote(host) == host && tui.composerRemote(nil) == nil, "hc composer: a TUI pane uploads to the session's host")
    check(hc.composerRemote(host) == nil, "hc composer: an hc pane never uploads to the TUI's host")
}

/// One answer per prompt across the card, the notification buttons and the hub (`ApprovalSender.answer`). On hc a
/// `terminal.keys` past its timeout may still land: the pane stays in flight until the prompt leaves the screen.
@MainActor func checksHCAnswerOnce() async throws {
    let fake = FakeHC(path: fakeHCPath())
    try fake.start()
    defer { fake.kill() }
    fake.terminals = [FakeHC.terminal("t1", status: "blocked", kind: "permission")]
    let prompt = "Bash command\n  rm -rf build\nDo you want to proceed?\n❯ 1. Yes\n  2. No"
    fake.setScreen("t1", prompt)
    let hc = HCClient(session: HCSession(socketPath: fake.path), callTimeout: 0.4)
    guard let s = await hc.approvalState(paneId: "hc:t1", agentKind: "claude", blocked: true, pending: nil),
          let id = ApprovalNotice.promptIDs(s, writes: true).last else { check(false, "answer once: the prompt has an id"); return }
    func keys() -> Int { fake.calls.filter { $0.method == "terminal.keys" }.count }
    let sender = ApprovalSender()
    func answer() async -> ApprovalAnswer {
        await sender.answer(paneId: "hc:t1", via: hc, prompt: id, allow: true, agentKind: "claude", blocked: true, pending: nil,
                            poll: .milliseconds(100), maxWait: .seconds(5))
    }

    // Two at once (card and notification): one goes out, the other is refused before any read.
    fake.keysDelay = 0.2
    async let a = answer()
    async let b = answer()
    let both = await [a, b]
    check(both.filter { $0 == .sent }.count == 1 && both.contains(.busy) && keys() == 1, "answer once: two presses, one answer (\(both), \(keys()) keys)")

    // Sent: still in flight while the prompt is on screen (a redraw lag must not let a second "1" through).
    check(sender.isAnswering("hc:t1"), "answer once: sent, in flight until the prompt leaves the screen")
    fake.setScreen("t1", "> done")
    for _ in 0..<30 where sender.isAnswering("hc:t1") { try await Task.sleep(for: .milliseconds(100)) }
    check(!sender.isAnswering("hc:t1"), "answer once: the prompt gone ends the flight after a send")
    fake.setScreen("t1", prompt)

    // A timeout on hc: unconfirmed, and the pane stays in flight while the prompt is on screen.
    fake.keysDelay = 1.0
    let n = keys()
    let late = await answer()
    check(late == .unconfirmed && sender.isAnswering("hc:t1"), "answer once: hc timeout = unconfirmed, still in flight (\(late))")
    let again = await answer()
    check(again == .busy && keys() == n + 1, "answer once: a second press while unconfirmed sends nothing (\(again))")
    fake.setScreen("t1", "> done")
    for _ in 0..<30 where sender.isAnswering("hc:t1") { try await Task.sleep(for: .milliseconds(100)) }
    check(!sender.isAnswering("hc:t1"), "answer once: the prompt gone ends the flight")
    fake.keysDelay = 0
}