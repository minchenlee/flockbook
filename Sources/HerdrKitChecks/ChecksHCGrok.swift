import Foundation
@testable import HerdrKit

// Grok Build and the remote hub on hc panes: the kit paths the app and the hub call with an hc pane id. FakeHC only.
@MainActor func checksHCGrok() async throws {
    let fake = FakeHC(path: fakeHCPath())
    try fake.start()
    defer { fake.kill() }
    let hc = HCClient(session: HCSession(socketPath: fake.path))
    func keys() -> [[String]] { fake.calls.filter { $0.method == "terminal.keys" || $0.method == "terminal.send" }
        .map { ($0.params["keys"] as? [String]) ?? [($0.params["text"] as? String) ?? "?"] } }

    // A Grok Build terminal is a Grok agent, run directly or through its node wrapper.
    fake.terminals = [FakeHC.terminal("t1", argv: ["grok"], status: "blocked"),
                      FakeHC.terminal("t2", argv: ["node", "/opt/grok/bin/grok"], status: "working")]
    let ws = try await hc.snapshot()
    let kinds = ws.flatMap(\.tabs).flatMap(\.panes).compactMap(\.agent).map(\.kind).sorted()
    check(kinds == ["grok", "grok"], "hc grok: grok terminals are Grok agents (\(kinds))")

    // Its permission box on an hc screen: the card, and Allow answers with Enter on "Yes, proceed".
    let rows = ["Yes, and don't ask again for anything (always-approve mode)", "Yes, proceed", "No, reject (type to add feedback)", "Never allow: echo"]
    func box(focus: Int) -> String {
        var l = ["  ◆ Write hi into out.txt… 3.0s", "", "  ┃", "  ┃  ›  Write hi into out.txt", "  ┃  │ echo hi > out.txt", "  ┃  ← → narrow scope", "  ┃"]
        for (i, r) in rows.enumerated() { l.append("  ┃  \(i + 1) (\(i + 1 == focus ? "●" : "○")) \(r)") }
        return (l + ["  ┃", "", "  1/4:select  │  Tab:next option  │  ←/→:scope  │  Ctrl+o:always-approve  │  Ctrl+c:cancel  │  Esc:scrollback"]).joined(separator: "\n")
    }
    fake.setScreen("t1", box(focus: 2))
    check(await hc.checkPrompt(paneId: "hc:t1"), "hc grok: the box on the hc screen is a prompt")
    guard let s = await hc.approvalState(paneId: "hc:t1", agentKind: "grok", blocked: true, pending: nil),
          let id = ApprovalNotice.promptIDs(s, writes: true).last else { check(false, "hc grok: the box has an id"); return }
    var n = keys().count
    let allowed = try await hc.respondIfOpen(paneId: "hc:t1", prompt: id, allow: true, agentKind: "grok", blocked: true, pending: nil)
    check(allowed && Array(keys().dropFirst(n)) == [["enter"]], "hc grok: Allow = enter through terminal.keys (\(Array(keys().dropFirst(n))))")
    // Deny: Down to "No, reject", then Enter once the focus is there.
    let focused = box(focus: 3)
    fake.onKeys = { addr, k in if addr == "t1", k == ["down"] { fake.setScreen("t1", focused) } }
    fake.setScreen("t1", box(focus: 2))
    n = keys().count
    let denied = try await hc.respondIfOpen(paneId: "hc:t1", prompt: id, allow: false, agentKind: "grok", blocked: true, pending: nil)
    check(denied && Array(keys().dropFirst(n)) == [["down"], ["enter"]], "hc grok: Deny = down, enter (\(Array(keys().dropFirst(n))))")
    fake.onKeys = nil
    // Another box on screen: the bound prompt is gone, nothing is sent.
    fake.setScreen("t1", box(focus: 2).replacingOccurrences(of: "echo hi > out.txt", with: "rm -rf out"))
    n = keys().count
    let stale = try await hc.respondIfOpen(paneId: "hc:t1", prompt: id, allow: true, agentKind: "grok", blocked: true, pending: nil)
    check(!stale && keys().count == n, "hc grok: another box: nothing sent")

    // The hub on hc: a device's gate is asked before the keys; a refusal sends nothing.
    fake.setScreen("t1", box(focus: 2))
    n = keys().count
    let refused = try await hc.respondIfOpen(paneId: "hc:t1", prompt: id, allow: true, agentKind: "grok", blocked: true, pending: nil,
                                             gate: KeyGate(agent: nil, allowed: { false }))
    check(!refused && keys().count == n, "hc hub: a gate that says no sends nothing")
    let yes = await hc.passes(KeyGate(agent: nil, allowed: { true }), paneId: "hc:t2")
    let no = await hc.passes(KeyGate(agent: nil, allowed: { false }), paneId: "hc:t2")
    check(yes && !no, "hc hub: passes asks the device's gate")
    // A gate bound to an agent without a session id fails closed (nothing stable to bind to).
    let seen = Agent(name: "g", kind: "grok", status: .working, sessionId: nil, title: "", backend: .hc)
    check(!(await hc.passes(KeyGate(agent: seen, allowed: { true }), paneId: "hc:t2")), "hc hub: a gate agent with no session fails closed")
    // Text from a phone: only when the agent is not blocked and nothing on screen takes keys.
    fake.setScreen("t2", "  ╭──────╮\n  │ ❯    │\n  ╰──────╯\n  Shift+Tab:mode  │  Ctrl+.:shortcuts")
    check(await hc.readyForText(paneId: "hc:t2"), "hc hub: a working grok with an empty box takes text")
    check(!(await hc.readyForText(paneId: "hc:t1")), "hc hub: a blocked grok does not take text")
    fake.setScreen("t2", box(focus: 2))
    check(!(await hc.readyForText(paneId: "hc:t2")), "hc hub: a box on screen does not take text")
}
